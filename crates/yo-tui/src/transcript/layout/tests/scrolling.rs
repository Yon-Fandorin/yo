use super::{
    TranscriptLayoutConfig, TranscriptScrollCommand, TranscriptState, TranscriptViewMode,
    TranscriptViewState, id, render_into, rendered_row,
};
use crate::surface::Size;

fn long_transcript() -> TranscriptState {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "0\n1\n2\n3\n4\n5\n6".into())
        .expect("unique user item");
    transcript
}

// 기본 FollowTail은 마지막 행을 붙잡고 새 content가 추가되어도 tail을 계속 보여준다.
#[test]
fn follows_the_tail_by_default() {
    let mut transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    let (surface, frame) = render_into(&transcript, Size::new(8, 3), &config, &mut state, None);
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(rendered_row(&surface, 0), "  4");
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);

    transcript
        .push_user(id(2), "tail".into())
        .expect("unique user item");
    let (surface, frame) = render_into(&transcript, Size::new(8, 3), &config, &mut state, None);
    assert_eq!(frame.first_visible_row, 7);
    assert_eq!(rendered_row(&surface, 2), "❯ tail");
}

// PageUp은 높이보다 한 행 적게 이동해 이전 화면의 첫 행을 다음 화면 끝에 남긴다.
#[test]
fn page_up_preserves_one_row_of_context() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    let (surface, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageUp),
    );

    assert_eq!(frame.first_visible_row, 2);
    assert_eq!(rendered_row(&surface, 0), "  2");
    assert_eq!(rendered_row(&surface, 2), "  4");
    assert_eq!(state.mode(), TranscriptViewMode::Detached);
}

// 아래로 tail까지 이동하면 FollowTail로 복귀해 이후 streaming 증가를 자동으로 따라간다.
#[test]
fn moving_down_to_the_tail_resumes_following() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageDown),
    );
    assert_eq!(frame.first_visible_row, 2);
    assert_eq!(state.mode(), TranscriptViewMode::Detached);

    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageDown),
    );
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
}

// 높이가 1인 화면도 PageDown을 0행이 아니라 1행 이동으로 해석한다.
#[test]
fn one_row_page_moves_by_one_line() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    render_into(
        &transcript,
        Size::new(8, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageDown),
    );

    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(state.mode(), TranscriptViewMode::Detached);
}

// 한 행 이동 명령과 tail 점프도 key가 아닌 동일한 의미 명령으로 상태를 전환한다.
#[test]
fn line_commands_and_tail_jump_share_the_semantic_scroll_path() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineUp),
    );
    assert_eq!(frame.first_visible_row, 3);
    assert_eq!(state.mode(), TranscriptViewMode::Detached);

    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineDown),
    );
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);

    render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToTail),
    );
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
}

// 내용이 화면에 모두 보여 위로 이동할 수 없으면 no-op 명령이 tail 추적을 끊지 않는다.
#[test]
fn no_op_upward_commands_keep_following_short_content() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "short".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default();

    for command in [
        TranscriptScrollCommand::LineUp,
        TranscriptScrollCommand::PageUp,
    ] {
        let mut state = TranscriptViewState::default();
        let (_, frame) = render_into(
            &transcript,
            Size::new(8, 3),
            &config,
            &mut state,
            Some(command),
        );

        assert_eq!(frame.first_visible_row, 0);
        assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
    }
}

// assistant 앞의 한 separator 행만 보이는 viewport는 아직 그 item을 표시하지 않았으므로
// 다음 item을 context로 잡지 않고, 실제 marker 행에 도달한 뒤에만 해당 ID를 보고한다.
#[test]
fn one_row_assistant_separator_has_no_following_item_context() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "first".into())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "second")
        .expect("streaming assistant");
    transcript.finalize(id(2)).expect("final assistant");
    let config = TranscriptLayoutConfig::default();
    let mut state = TranscriptViewState::default();

    let (separator, frame) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineUp),
    );
    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(rendered_row(&separator, 0), "");
    assert_eq!(frame.context_item, None);

    let (_, frame) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineDown),
    );
    assert_eq!(frame.first_visible_row, 2);
    assert_eq!(frame.context_item, Some(id(2)));
}

// user 앞의 두 separator 행은 각각 독립적인 한 행 viewport에서도 context가 없고,
// 이전·다음 item의 실제 marker 행만 정확한 item ID에 속한다.
#[test]
fn two_row_user_separator_excludes_both_blank_boundary_rows() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).expect("unique assistant");
    transcript
        .append_text(id(1), "first")
        .expect("streaming assistant");
    transcript.finalize(id(1)).expect("final assistant");
    transcript
        .push_user(id(2), "second".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default();
    let mut state = TranscriptViewState::default();

    let (_, first) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    assert_eq!(first.context_item, Some(id(1)));

    for expected_row in [1, 2] {
        let (separator, frame) = render_into(
            &transcript,
            Size::new(12, 1),
            &config,
            &mut state,
            Some(TranscriptScrollCommand::LineDown),
        );
        assert_eq!(frame.first_visible_row, expected_row);
        assert_eq!(rendered_row(&separator, 0), "");
        assert_eq!(frame.context_item, None);
    }

    let (_, second) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineDown),
    );
    assert_eq!(second.first_visible_row, 3);
    assert_eq!(second.context_item, Some(id(2)));
}

// resize는 FollowTail을 새 tail에 맞추고 Detached offset은 clamp하되 읽기 의도는 유지한다.
#[test]
fn resize_reflows_following_and_clamps_detached_state() {
    let transcript = long_transcript();
    let config = TranscriptLayoutConfig::default();
    let mut following = TranscriptViewState::default();

    let (_, frame) = render_into(&transcript, Size::new(8, 4), &config, &mut following, None);
    assert_eq!(frame.first_visible_row, 3);
    assert_eq!(following.mode(), TranscriptViewMode::FollowTail);

    let mut detached = TranscriptViewState::default();
    render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut detached,
        Some(TranscriptScrollCommand::LineUp),
    );
    let (_, frame) = render_into(&transcript, Size::new(8, 8), &config, &mut detached, None);
    assert_eq!(frame.first_visible_row, 0);
    assert_eq!(detached.mode(), TranscriptViewMode::Detached);
}

// 누적 높이의 u16 마지막 값·첫 초과·9만 행에서도 각 메시지 경계와 사용자 배경을 유지하고
// Home/End가 논리 위치를 화면의 작은 좌표로 변환한다.
#[test]
fn accumulated_rows_cross_u16_without_wrapping_content_or_scroll_positions() {
    use super::styles;
    use crate::{
        surface::{Color, Point, Rect, Surface},
        transcript::{paint_prepared, prepare},
    };
    for total in [65_535_usize, 65_536, 98_308] {
        let mut transcript = TranscriptState::new();
        for (index, rows) in [22_000, 22_000, total - 44_004].into_iter().enumerate() {
            transcript
                .push_user(
                    id(index as u64 + 1),
                    format!("{}last{index}", "x\n".repeat(rows - 1)),
                )
                .unwrap();
        }
        let prepared = prepare(&transcript, 12, &TranscriptLayoutConfig::default()).unwrap();
        assert_eq!(prepared.content_height(), total);
        let mut palette = styles();
        palette.user_body.background = Color::Indexed(236);
        let mut state = TranscriptViewState::default();
        for command in [
            None,
            Some(TranscriptScrollCommand::JumpToStart),
            Some(TranscriptScrollCommand::JumpToTail),
        ] {
            let size = Size::new(12, 3);
            let mut surface = Surface::new(size).unwrap();
            let frame = paint_prepared(
                prepared.clone(),
                &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
                palette,
                &mut state,
                command,
            )
            .unwrap();
            assert_eq!(frame.content_height, total);
            if command == Some(TranscriptScrollCommand::JumpToStart) {
                assert_eq!(frame.first_visible_row, 0);
                assert_eq!(frame.context_item, Some(id(1)));
                assert!(rendered_row(&surface, 0).contains("❯ x"));
            } else {
                assert_eq!(frame.first_visible_row, total - 3);
                assert_eq!(frame.context_item, Some(id(3)));
                assert_eq!(rendered_row(&surface, 2), "  last2");
            }
            assert_eq!(
                surface.cell(Point::new(11, 2)).unwrap().style().background,
                Color::Indexed(236)
            );
        }
    }
}

// 실제 viewport 크기는 u16 범위에 두고 usize 끝 근처의 문서 위치도 감기지 않게 변환한다.
#[test]
fn document_positions_near_usize_limit_translate_only_visible_rows() {
    use std::num::NonZeroU16;

    use super::super::VisibleRows;
    use crate::surface::Point;
    let visible = VisibleRows::resolve_commands(
        usize::MAX,
        NonZeroU16::new(3).unwrap(),
        TranscriptViewState::default(),
        &[TranscriptScrollCommand::JumpToTail],
        &[],
    );
    assert_eq!(visible.first(), usize::MAX - 3);
    assert_eq!(visible.translate(7, usize::MAX - 1), Point::new(7, 2));
    let scrolled = VisibleRows::resolve_commands(
        usize::MAX,
        NonZeroU16::new(3).unwrap(),
        visible.next_state(),
        &[
            TranscriptScrollCommand::LineDown,
            TranscriptScrollCommand::PageUp,
        ],
        &[],
    );
    assert_eq!(scrolled.first(), usize::MAX - 5);
}

// 65,535행 이후의 코드 배경·메타 행·원본 이미지도 같은 논리 offset을 거쳐 viewport에 놓인다.
#[test]
fn diff_bands_and_rasters_after_large_history_use_local_surface_coordinates() {
    use std::io::Cursor;

    use base64::{Engine, engine::general_purpose::STANDARD};
    use image::{ImageFormat, RgbImage};

    use super::styles;
    use crate::{
        surface::{CellContent, Color, Point, Rect, Surface},
        transcript::{paint_prepared, prepare},
    };
    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(2, 2)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let mut transcript = TranscriptState::new();
    for index in 1..=2 {
        transcript
            .push_user(id(index), "x\n".repeat(32_767))
            .unwrap();
    }
    transcript.start_markdown_assistant(id(3)).unwrap();
    transcript
        .append_text(
            id(3),
            &format!(
                "```diff\n+green\n-red\n@@ hunk\n```\n\n![pixel](data:image/png;base64,{})",
                STANDARD.encode(encoded.into_inner())
            ),
        )
        .unwrap();
    let prepared = prepare(&transcript, 32, &TranscriptLayoutConfig::default()).unwrap();
    assert_eq!(prepared.layout.rasters.len(), 1);
    let image_row = prepared.layout.rasters[0].0;
    assert!(image_row > usize::from(u16::MAX));
    let mut palette = styles();
    palette.markdown.rich_media = true;
    palette.markdown.code.foreground = Color::Indexed(7);
    palette.markdown.pixel_color_capability = Color::Indexed(7);
    palette.markdown.diff_added.background = Color::Indexed(22);
    palette.markdown.diff_removed.background = Color::Indexed(52);
    palette.markdown.diff_meta.background = Color::Indexed(17);
    let size = Size::new(32, 40);
    let mut surface = Surface::new(size).unwrap();
    let frame = paint_prepared(
        prepared,
        &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
        palette,
        &mut TranscriptViewState::default(),
        None,
    )
    .unwrap();
    assert_eq!(frame.context_item, Some(id(3)));
    for (marker, background) in [
        ("+", Color::Indexed(22)),
        ("-", Color::Indexed(52)),
        ("@", Color::Indexed(17)),
    ] {
        let row = (0..40).find(|row| (0..32).any(|column| matches!(surface.cell(Point::new(column,*row)).unwrap().content(), CellContent::Grapheme {text,..} if text.as_ref() == marker))).unwrap();
        assert_eq!(
            surface
                .cell(Point::new(31, row))
                .unwrap()
                .style()
                .background,
            background
        );
    }
    assert_eq!(surface.rasters.len(), 1);
    assert_eq!(
        usize::from(surface.rasters[0].area.origin.y),
        image_row - frame.first_visible_row
    );
}

// 항목 이동은 줄 수 대신 현재 폭의 실제 시작 행을 사용하고 마지막에서 tail 추적을 재개한다.
#[test]
fn item_navigation_uses_reflowed_boundaries_and_resumes_tail() {
    let mut transcript = TranscriptState::new();
    for index in 1..=3 {
        transcript
            .push_user(
                id(index),
                format!("ITEM{index} 한글 text\nsecond\nthird\nfourth\nfifth"),
            )
            .unwrap();
    }
    let config = TranscriptLayoutConfig::default();
    for width in [30, 12, 30] {
        let mut state = TranscriptViewState::default();
        let size = Size::new(width, 3);
        let (_, frame) = render_into(
            &transcript,
            size,
            &config,
            &mut state,
            Some(TranscriptScrollCommand::JumpToStart),
        );
        assert_eq!(frame.context_item, Some(id(1)));
        for index in [2, 3] {
            let (surface, frame) = render_into(
                &transcript,
                size,
                &config,
                &mut state,
                Some(TranscriptScrollCommand::NextItem),
            );
            assert_eq!(frame.context_item, Some(id(index)));
            assert!(rendered_row(&surface, 0).contains(&format!("ITEM{index}")));
        }
        let (_, frame) = render_into(
            &transcript,
            size,
            &config,
            &mut state,
            Some(TranscriptScrollCommand::NextItem),
        );
        assert_eq!(frame.context_item, Some(id(3)));
        assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
        for index in [3, 2, 1, 1] {
            let (surface, frame) = render_into(
                &transcript,
                size,
                &config,
                &mut state,
                Some(TranscriptScrollCommand::PreviousItem),
            );
            assert_eq!(frame.context_item, Some(id(index)));
            assert!(rendered_row(&surface, 0).contains(&format!("ITEM{index}")));
            assert_eq!(state.mode(), TranscriptViewMode::Detached);
        }
    }
}
