use super::support::*;

// 음수 막대는 0선 왼쪽, 양수는 오른쪽에 표시하고 숫자를 그대로 남긴다.
// NaN·불완전한 데이터는 잘못된 차트를 만들지 않고 원문으로 되돌린다.
#[test]
fn charts_preserve_values_zero_direction_and_invalid_source() {
    let rendered = prepare(
        "```chart\nDown: -8\nZero: 0\nUp: 4\n```",
        NonZeroU16::new(20).unwrap(),
    )
    .unwrap();
    let lines = rows(&rendered);
    assert!(lines.iter().any(|line| line.contains("Down  -8")));
    assert!(lines.iter().any(|line| line.contains("Zero  0")));
    assert!(lines.iter().any(|line| line.contains("█│")));
    assert!(lines.iter().any(|line| line.contains("│█")));
    let axes: Vec<_> = rendered
        .glyphs
        .iter()
        .filter(|g| g.grapheme.as_str() == "│")
        .map(|g| g.point.x)
        .collect();
    assert_eq!(axes.len(), 3);
    assert!(axes.iter().all(|x| *x == axes[0]));
    for source in [
        "```chart\nValue: NaN\n```",
        "```chart\nValue: pending\n```",
        "```sparkline\n\n```",
    ] {
        let rendered = prepare(source, NonZeroU16::new(20).unwrap()).unwrap();
        assert!(
            !rendered
                .glyphs
                .iter()
                .any(|g| g.decoration.role == Role::Chart)
        );
    }
    let trend = prepare("```sparkline\n5 5 5\n```", NonZeroU16::new(20).unwrap()).unwrap();
    assert!(rows(&trend).iter().any(|line| line.contains("▅▅▅")));
}

// 선 차트는 실제 데이터 점들을 연결하고 축·범위를 제공한다. 좁은 폭은 sparkline으로
// 내려가도 값은 보존되고 색상/문자 fallback과 무관하게 범위를 벗어나지 않는다.
#[test]
fn line_chart_is_connected_and_responsive() {
    for width in [12, 40, 88] {
        let prepared = prepare(
            "```linechart\n12 8 16 10 24\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        assert!(
            prepared
                .glyphs
                .iter()
                .all(|g| g.point.x + g.grapheme.width().get() <= width)
        );
        if width >= 40 {
            assert!(prepared.glyphs.iter().any(|g| {
                g.grapheme
                    .as_str()
                    .chars()
                    .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
            }));
            assert!(rows(&prepared).join("\n").contains("1 → 5 samples"));
        }
    }
}

// 표현 가능한 최소 양수도 0과 다른 높이로 표시하고 큰 지수 값에서도 축을 같은 열에 맞춘다.
#[test]
fn charts_preserve_extreme_numeric_ranges_and_axis_alignment() {
    for source in ["0 5e-324", "-5e-324 0", "-1e308 1e308"] {
        let chart = prepare(
            &format!("```sparkline\n{source}\n```"),
            NonZeroU16::new(24).unwrap(),
        )
        .unwrap();
        assert!(
            rows(&chart).iter().any(|row| row.contains("▁█")),
            "{:?}",
            rows(&chart)
        );
        let text = rows(&chart)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        assert!(
            text.contains(&format!(
                "last{}",
                source.split_whitespace().last().unwrap()
            )),
            "{text}"
        );
    }
    for width in [18, 24, 40] {
        let chart = prepare(
            "```linechart\n0 1e308 -1e308\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let axes: Vec<_> = chart
            .glyphs
            .iter()
            .filter(|glyph| glyph.grapheme.as_str() == "│")
            .map(|glyph| glyph.point)
            .collect();
        assert_eq!(axes.len(), 6, "{:?}", rows(&chart));
        assert!(axes.iter().all(|point| point.x == axes[0].x), "{axes:?}");
        assert!(
            axes.windows(2).all(|pair| pair[1].y == pair[0].y + 1),
            "{axes:?}"
        );
    }
}

// 막대 길이는 수치로 계산하되 지수·부호·소수 자릿수는 입력 표기를 그대로 보여준다.
#[test]
fn bar_charts_keep_original_numeric_notation() {
    for width in [18, 80] {
        let chart = prepare(
            "```chart\nTiny: 5e-324\nDecimal: 1.2300\nSigned: +2.00\nZero: -0.0\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        for literal in ["5e-324", "1.2300", "+2.00", "-0.0"] {
            assert!(text.contains(literal), "{text}");
        }
        assert!(!text.contains("0000000000"), "{text}");
    }
}

// 계단 차트는 다음 관측까지 이전 값을 수평 유지하고 마지막 위치에서만 수직으로 바뀐다.
#[test]
fn step_chart_holds_previous_value_and_preserves_narrow_source() {
    for width in [12, 40, 88] {
        let prepared = prepare("```stepchart\n0 10\n```", NonZeroU16::new(width).unwrap()).unwrap();
        assert!(
            prepared
                .glyphs
                .iter()
                .all(|g| g.point.x + g.grapheme.width().get() <= width)
        );
        let text = rows(&prepared).join("\n");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains("Values:010"),
            "{text}"
        );
        if width >= 40 {
            let cells: Vec<_> = prepared
                .glyphs
                .iter()
                .filter_map(|g| {
                    let c = g.grapheme.as_str().chars().next()?;
                    ('\u{2800}'..='\u{28ff}')
                        .contains(&c)
                        .then_some((g.point.x, g.point.y, c))
                })
                .collect();
            assert!(!cells.is_empty());
            let right = cells.iter().map(|c| c.0).max().unwrap();
            let bottom = cells.iter().map(|c| c.1).max().unwrap();
            assert!(
                cells
                    .iter()
                    .any(|&(x, y, c)| x < right && y == bottom && c != '\u{2800}')
            );
            assert!(
                cells
                    .iter()
                    .any(|&(x, y, c)| x == right && y < bottom && c != '\u{2800}')
            );
            assert!(
                cells
                    .iter()
                    .all(|&(x, y, c)| c == '\u{2800}' || x == right || y == bottom),
                "{text}"
            );
        }
    }
    for source in ["NaN", "", "1 infinity"] {
        let prepared = prepare(
            &format!("```stepchart\n{source}\n```"),
            NonZeroU16::new(40).unwrap(),
        )
        .unwrap();
        assert!(rows(&prepared).join("\n").contains("Chart data incomplete"));
    }
}

// 구간 경계 값은 오른쪽 구간에 들어가고 최댓값은 마지막 구간에 포함되며 좁은 폭에도 개수를
// 보존한다.
#[test]
fn histogram_counts_boundaries_and_preserves_numeric_source() {
    for width in [80, 24, 12, 80] {
        let chart = prepare(
            "```histogram\nbins: 4\n0 1 2 2 3 4.00\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let rendered = rows(&chart);
        let text = rendered.join(" ").split_whitespace().collect::<String>();
        assert!(text.contains("6samples·4bins·count"), "{text}");
        assert!(text.contains("Values:012234.00"), "{text}");
        if width >= 24 {
            for (interval, count) in [
                ("[0, 1)", "1"),
                ("[1, 2)", "1"),
                ("[2, 3)", "2"),
                ("[3, 4]", "2"),
            ] {
                let row = rendered.iter().find(|row| row.contains(interval)).unwrap();
                assert!(row.trim_end().ends_with(count), "{row}");
            }
        }
        assert!(
            chart
                .glyphs
                .iter()
                .any(|glyph| glyph.decoration.role == Role::Bar)
        );
    }
}

// bin·표본 한도의 첫 초과는 원문으로 돌아가고 상수·극단 범위도 유한한 구간으로 표시한다.
#[test]
fn histogram_limits_constant_and_extreme_values() {
    for (values, expected) in [
        ("bins: 32\n0 1", "32bins"),
        ("5 5 5", "1bin"),
        ("0 5e-324", "1bin"),
        ("bins: 2\n-1e308 0 1e308", "2bins"),
    ] {
        let chart = prepare(
            &format!("```histogram\n{values}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains("NaN") && !text.contains("inf"));
        assert!(!text.contains("0000000000"), "{text}");
    }
    let accepted = iter::repeat_n("1", 64).collect::<Vec<_>>().join(" ");
    for (source, valid) in [
        (accepted.clone(), true),
        (format!("{accepted} 1"), false),
        ("bins: 33\n0 1".into(), false),
        ("bins: 0\n0 1".into(), false),
        ("NaN".into(), false),
        ("bins: 2".into(), false),
    ] {
        let chart = prepare(
            &format!("```histogram\n{source}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        assert_eq!(text.contains("Chart data incomplete"), !valid, "{text}");
        if !valid {
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains(&source.split_whitespace().collect::<String>()),
                "{text}"
            );
        }
    }
}

// 산점도는 입력 순서가 아닌 X 값 간격으로 점을 놓고 연결선을 만들지 않으며 좁은 폭에는 좌표를
// 남긴다.
#[test]
fn scatter_uses_numeric_x_spacing_and_unconnected_points() {
    for width in [80, 24, 12, 80] {
        let chart = prepare(
            "```scatterchart\n0,0 1,5 10,10.00\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let text = rows(&chart)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        assert!(text.contains("X0→10"), "{text}");
        assert!(text.contains("Y0→10·3points"), "{text}");
        assert!(text.contains("Points:0,01,510,10.00"), "{text}");
        let dots: Vec<_> = chart
            .glyphs
            .iter()
            .filter_map(|glyph| {
                let character = glyph.grapheme.as_str().chars().next()? as u32;
                (0x2801..=0x28ff)
                    .contains(&character)
                    .then(|| (glyph.point, character - 0x2800))
            })
            .collect();
        if width >= 24 {
            assert_eq!(
                dots.iter().map(|(_, mask)| mask.count_ones()).sum::<u32>(),
                3
            );
            assert_eq!(dots.len(), 3);
            let top = dots.iter().min_by_key(|(point, _)| point.y).unwrap().0;
            let bottom = dots.iter().max_by_key(|(point, _)| point.y).unwrap().0;
            let middle = dots
                .iter()
                .find(|(point, _)| point.y != top.y && point.y != bottom.y)
                .unwrap()
                .0;
            assert!(top.x > bottom.x);
            assert!(middle.x - bottom.x < (top.x - bottom.x) / 2);
        } else {
            assert!(dots.is_empty());
            assert!(text.contains("showingcoordinates"));
        }
    }
}

// 단일 점·상수 축·극단 좌표를 지원하고 좌표 한쪽 오류나 표본 한도의 첫 초과는 원문으로 돌린다.
#[test]
fn scatter_handles_constant_axes_and_rejects_invalid_coordinates() {
    for source in [
        "5,10",
        "5,10 5,20 5,30",
        "0,0 5e-324,5e-324",
        "-1e308,-1e308 1e308,1e308",
    ] {
        let chart = prepare(
            &format!("```scatterchart\n{source}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        assert!(
            !text.contains("incomplete") && !text.contains("NaN") && !text.contains("inf"),
            "{text}"
        );
        assert!(chart.glyphs.iter().any(|glyph| {
            glyph
                .grapheme
                .as_str()
                .chars()
                .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
        }));
    }
    let accepted = iter::repeat_n("0,1", 64).collect::<Vec<_>>().join(" ");
    for (source, valid) in [
        (accepted.clone(), true),
        (format!("{accepted} 0,1"), false),
        ("NaN,1".into(), false),
        ("1,inf".into(), false),
        ("1,2,3".into(), false),
        ("1,".into(), false),
        ("1 2".into(), false),
    ] {
        let chart = prepare(
            &format!("```scatterchart\n{source}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        assert_eq!(text.contains("Chart data incomplete"), !valid, "{text}");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains(&source.split_whitespace().collect::<String>()),
            "{text}"
        );
    }
}

// 차트별 높이는 실제 축 행 수에 반영되고 최소·최대 경계와 좁은 폭 원문 수치를 보존한다.
#[test]
fn chart_height_controls_plot_rows_and_rejects_first_excess() {
    for (kind, values) in [
        ("linechart", "1 5 2"),
        ("stepchart", "1 5 2"),
        ("scatterchart", "0,1 1,5 2,2"),
    ] {
        for height in [2, 3, 6, 16] {
            for width in [80, 24, 12] {
                let chart = prepare(
                    &format!("```{kind}\nheight: {height}\n{values}\n```"),
                    NonZeroU16::new(width).unwrap(),
                )
                .unwrap();
                let axes = chart
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.grapheme.as_str() == "│")
                    .count();
                assert_eq!(axes, if width >= 24 { height } else { 0 });
                let text = rows(&chart)
                    .join(" ")
                    .split_whitespace()
                    .collect::<String>();
                assert!(!text.contains("height:"), "{text}");
                assert!(
                    text.contains(&values.split_whitespace().collect::<String>()),
                    "{text}"
                );
                assert!(!text.contains("incomplete"));
            }
        }
        for height in ["1", "17", "-1", "2.5", "pending"] {
            let source = format!("height: {height}\n{values}");
            let chart = prepare(
                &format!("```{kind}\n{source}\n```"),
                NonZeroU16::new(80).unwrap(),
            )
            .unwrap();
            let text = rows(&chart).join(" ");
            assert!(text.contains("Chart data incomplete"), "{text}");
            assert!(text.contains(&format!("height: {height}")), "{text}");
        }
    }
}

// 여러 계열은 공통 Y 축에서 비교하고 번호·범례·원래 수치를 좁은 폭에서도 유지한다.
#[test]
fn named_charts_share_axes_and_keep_numbered_legends_and_values() {
    for kind in ["linechart", "stepchart", "scatterchart"] {
        let data = if kind == "scatterchart" {
            "Low: 0,0 1,0\nHigh: 100,10"
        } else {
            "Low: 0 0\nHigh: 10 10"
        };
        for width in [80, 24, 12] {
            let rendered = prepare(
                &format!("```{kind}\nheight: 4\n{data}\n```"),
                NonZeroU16::new(width).unwrap(),
            )
            .unwrap();
            let text = rows(&rendered).join("\n");
            let flat = text.split_whitespace().collect::<String>();
            assert!(
                flat.contains("[1]Low") && flat.contains("[2]High"),
                "{text}"
            );
            assert!(flat.contains("Y0→10"), "{text}");
            assert!(
                flat.contains(if kind == "scatterchart" {
                    "[1]Low:0,01,0"
                } else {
                    "[1]Low:00"
                }),
                "{text}"
            );
            if width >= 24 {
                let plot = rows(&rendered)
                    .into_iter()
                    .filter(|row| row.contains('│'))
                    .collect::<Vec<_>>();
                assert_eq!(plot.len(), 4);
                assert!(plot[0].split_once('│').unwrap().1.contains('2'), "{text}");
                assert!(plot[3].split_once('│').unwrap().1.contains('1'), "{text}");
                if kind == "scatterchart" {
                    assert!(flat.contains("X0→100"), "{text}");
                    assert!(plot[3].split_once('│').unwrap().1.ends_with(' '), "{text}");
                }
                let first_plot_row = rows(&rendered)
                    .iter()
                    .position(|row| row.contains('│'))
                    .unwrap() as u16;
                assert!(rendered.glyphs.iter().any(|g| g.point.y == first_plot_row
                    && g.grapheme.as_str() == "2"
                    && g.decoration.role == Role::ChartSeries(1)));
                assert!(
                    rendered
                        .glyphs
                        .iter()
                        .any(|g| g.decoration.role == Role::ChartSeries(1))
                );
            } else {
                assert!(flat.contains("Chartnarrowed"), "{text}");
            }
        }
    }
    let shared = prepare(
        "```linechart\nA: 1 2\nB: 1 2\n```",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    assert!(rows(&shared).join("\n").contains("× plotted overlap"));
    let mixed = prepare(
        "```linechart\nheight: 4\nA: 0 0\nB: 1 1\nC: 10 10\n```",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    let mixed_rows = rows(&mixed).join("\n");
    assert!(mixed_rows.contains("Mixed cells shown muted"));
    assert!(!mixed_rows.contains('×'));
    let ascii = MarkdownStyles {
        rich_media: false,
        ..MarkdownStyles::plain(Style::default())
    };
    assert_eq!(
        ascii
            .display_glyph(
                Decoration::role(Role::ChartSeries(1)),
                Grapheme::try_from("⠉").unwrap()
            )
            .as_str(),
        "*"
    );
    assert_eq!(
        ascii
            .display_glyph(
                Decoration::role(Role::Chart),
                Grapheme::try_from("×").unwrap()
            )
            .as_str(),
        "x"
    );
}

// 계열·이름·표본 상한의 첫 초과, 중복 이름, 불균일한 선 표본과 비유한 값은 전체 원문으로 돌아간다.
#[test]
fn named_chart_boundaries_reject_ambiguous_or_excess_data() {
    let samples = "1 ".repeat(64);
    let cases = [
        (
            (1..=4)
                .map(|i| format!("S{i}: {samples}"))
                .collect::<Vec<_>>()
                .join("\n"),
            true,
        ),
        (
            (1..=5)
                .map(|i| format!("S{i}: 1 2"))
                .collect::<Vec<_>>()
                .join("\n"),
            false,
        ),
        (format!("{}: 1 2", "n".repeat(64)), true),
        (format!("{}: 1 2", "n".repeat(65)), false),
        (format!("A: {}", "1 ".repeat(65)), false),
        ("A: 1 2\nA: 3 4".into(), false),
        ("A: 1 2\nB: 3".into(), false),
        ("A: NaN 2".into(), false),
        (": 1 2".into(), false),
        ("A: 1e308 -1e308\nB: 0 0".into(), true),
        ("A: 4\nB: 4".into(), true),
    ];
    for (data, valid) in cases {
        let rendered = prepare(
            &format!("```linechart\n{data}\n```"),
            NonZeroU16::new(40).unwrap(),
        )
        .unwrap();
        let text = rows(&rendered)
            .join("\n")
            .split_whitespace()
            .collect::<String>();
        assert_eq!(!text.contains("Chartdataincomplete"), valid, "{data}");
        if !valid {
            assert!(
                text.contains(&data.split_whitespace().collect::<String>()),
                "{data}"
            );
        }
    }
}
