use super::*;

fn width(value: u16) -> OutputWidth {
    OutputWidth::Bounded(NonZeroU16::new(value).unwrap())
}

fn spec<'a>(columns: &'a [Column<'a>]) -> ListSpec<'a> {
    ListSpec {
        columns,
        gap: NonZeroU16::new(2).unwrap(),
        heading_style: HeadingStyle::Plain,
    }
}

// 모든 열이 정확히 들어가는 폭에서는 접힌 상세 영역 없이 기존 표 형태를 유지해,
// 충분히 넓은 terminal에서 정보 밀도가 불필요하게 낮아지지 않습니다.
#[test]
fn exact_fit_keeps_every_column_inline() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "PATH",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Block,
            },
        },
    ];
    let rows = vec![vec!["a".to_owned(), "/yo".to_owned()]];

    assert_eq!(
        render_list(spec(&columns), &rows, width(8)).unwrap(),
        "ID  PATH\na   /yo\n"
    );
}

// 표보다 한 셀 좁아지면 같은 우선순위의 PATH 열 전체를 row 아래로 옮기고, 해당
// 폭에 label/value pair가 들어가지 않을 때만 값을 다음 줄로 내려 보존합니다.
#[test]
fn one_cell_short_folds_the_priority_group() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "PATH",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Block,
            },
        },
    ];
    let rows = vec![vec!["a".to_owned(), "/yo".to_owned()]];

    assert_eq!(
        render_list(spec(&columns), &rows, width(7)).unwrap(),
        "ID\na\nPATH\n  /yo\n"
    );
}

// 한글과 emoji를 글자 수가 아니라 terminal cell 폭으로 계산해야 접기 경계가 실제
// 화면과 일치하며, grapheme 중간을 쪼개지 않은 채 다음 줄로 넘깁니다.
#[test]
fn unicode_cell_width_controls_wrapping() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "PATH",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Block,
            },
        },
    ];
    let rows = vec![vec!["가".to_owned(), "가🙂A".to_owned()]];

    let output = render_list(spec(&columns), &rows, width(7)).unwrap();

    assert!(output.contains("  가🙂A\n"));
}

// 파이프와 파일 출력은 terminal 폭 추측에 따라 모양이 달라지지 않으며, 긴 값도 한
// logical row에 남아 후속 명령이 안정적으로 처리할 수 있습니다.
#[test]
fn unbounded_output_preserves_one_row_per_item() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "PATH",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Block,
            },
        },
    ];
    let rows = vec![vec!["a".to_owned(), "/very/long/path".to_owned()]];

    assert_eq!(
        render_list(spec(&columns), &rows, OutputWidth::Unbounded).unwrap(),
        "ID  PATH\na   /very/long/path\n"
    );
}

// 중간 폭에서는 공유 table header를 한 번만 유지하고, 짧은 접힌 항목은 완전한
// label/value pair 단위로 같은 줄에 채우며, 독립 block은 남은 전체 폭에 들어가면
// 자신의 한 줄을 사용하고 record 사이는 빈 줄로 분리합니다.
#[test]
fn folded_flow_packs_pairs_and_preserves_record_boundaries() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "VERSION",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Flow,
            },
        },
        Column {
            heading: "STATE",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Flow,
            },
        },
        Column {
            heading: "PATH",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Block,
            },
        },
    ];
    let rows = vec![vec!["a", "v1", "ok", "/one"], vec!["b", "v1", "ok", "/two"]]
        .into_iter()
        .map(|row| row.into_iter().map(str::to_owned).collect())
        .collect::<Vec<_>>();

    assert_eq!(
        render_list(spec(&columns), &rows, width(23)).unwrap(),
        "ID\na\nVERSION  v1  STATE  ok\nPATH  /one\n\nb\nVERSION  v1  STATE  ok\nPATH  /two\n"
    );
}

// 독립 block의 label/value pair가 접힌 영역의 전체 폭 안에 들어가면 불필요하게
// 세로로 쪼개지 않고 한 줄을 온전히 사용합니다.
#[test]
fn fitting_block_keeps_its_label_and_value_inline() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "PATH",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Block,
            },
        },
    ];
    let rows = vec![vec!["abcdefghij".to_owned(), "/work/yo".to_owned()]];

    assert_eq!(
        render_list(spec(&columns), &rows, width(15)).unwrap(),
        "ID\nabcdefghij\nPATH  /work/yo\n"
    );
}

// Flow pair 하나가 전체 폭보다 길면 일부만 옆에 남겨 모호하게 만들지 않고 해당
// 항목만 block으로 승격해 label과 wrapped value를 온전히 보존합니다.
#[test]
fn oversized_flow_pair_promotes_to_a_block() {
    let columns = [
        Column {
            heading: "ID",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "DETAIL",
            behavior: ColumnBehavior::Collapsible {
                priority: 1,
                continuation: ContinuationLayout::Flow,
            },
        },
    ];
    let rows = vec![vec!["a".to_owned(), "abcdefgh".to_owned()]];

    assert_eq!(
        render_list(spec(&columns), &rows, width(7)).unwrap(),
        "ID\na\nDETAIL\n  abcde\n  fgh\n"
    );
}

// 고정 열조차 한 줄에 들어가지 않는 아주 좁은 terminal은 데이터를 자르지 않고 모든
// 열을 라벨-값 세로 목록으로 바꿔, UUID와 상태를 계속 읽을 수 있게 합니다.
#[test]
fn very_narrow_width_uses_vertical_layout_without_truncation() {
    let columns = [
        Column {
            heading: "RESUME",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "STATUS",
            behavior: ColumnBehavior::Pinned,
        },
    ];
    let rows = vec![vec!["uuid".to_owned(), "available".to_owned()]];

    let output = render_list(spec(&columns), &rows, width(8)).unwrap();

    assert!(output.contains("RESUME\n  uuid\n"));
    assert!(output.contains("STATUS\n  availa\n  ble\n"));
}

// 1~7셀처럼 라벨보다도 좁은 폭에서는 들여쓰기를 점차 줄이고 라벨까지 개행해,
// ASCII로 표현 가능한 모든 물리 줄이 사용 가능한 폭을 넘지 않습니다.
#[test]
fn tiny_widths_adapt_label_and_value_indentation() {
    let columns = [Column {
        heading: "RESUME",
        behavior: ColumnBehavior::Pinned,
    }];
    let rows = vec![vec!["abc".to_owned()]];

    for available in 1..=7 {
        let output = render_list(spec(&columns), &rows, width(available)).unwrap();
        for line in output.lines() {
            assert!(
                cell_width(line).unwrap() <= usize::from(available),
                "{available}-cell output contains overwide line {line:?}"
            );
        }
    }
}

// 너비 1 terminal에는 2셀 한글 grapheme을 자르거나 거짓으로 맞출 수 없으므로,
// renderer는 데이터 손실 대신 물리적으로 표현 불가능한 폭을 명시적으로 거절합니다.
#[test]
fn atomic_wide_grapheme_reports_an_unrepresentable_width() {
    let columns = [Column {
        heading: "ID",
        behavior: ColumnBehavior::Pinned,
    }];
    let rows = vec![vec!["가".to_owned()]];

    assert_eq!(
        render_list(spec(&columns), &rows, width(1)).unwrap_err(),
        ListError::GraphemeExceedsWidth {
            grapheme_width: 2,
            width: 1,
        }
    );
}

// 굵은 heading은 terminal cell 폭 계산에는 영향을 주지 않는 ANSI 장식으로만 더해져,
// 표의 열 시작점은 plain heading과 같고 데이터 row에는 escape가 섞이지 않습니다.
#[test]
fn bold_heading_style_decorates_only_semantic_labels() {
    let columns = [Column {
        heading: "ID",
        behavior: ColumnBehavior::Pinned,
    }];
    let rows = vec![vec!["value".to_owned()]];
    let mut styled = spec(&columns);
    styled.heading_style = HeadingStyle::BoldAnsi;

    assert_eq!(
        render_list(styled, &rows, OutputWidth::Unbounded).unwrap(),
        "\u{1b}[1mID\u{1b}[0m\nvalue\n"
    );
}
