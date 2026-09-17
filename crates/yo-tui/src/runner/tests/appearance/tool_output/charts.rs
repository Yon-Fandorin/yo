use super::*;

// 차트 색상은 제목 accent와 독립적으로 적용되고 override 제거 시 다시 accent를 따른다.
#[test]
fn chart_color_is_independent_from_headings_and_preserves_reflowed_source() {
    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    let source = "# Chart heading\n\n```linechart\n1 5 2 8\n```\n\n```chart\nA: 10\nB: 20\n```";
    let accent = Color::Rgb {
        red: 12,
        green: 180,
        blue: 160,
    };
    let chart = Color::Rgb {
        red: 230,
        green: 160,
        blue: 40,
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source.to_owned()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for custom in [true, false, true] {
        let mut overrides =
            ThemeOverrides::default().with_color(ThemeRole::Accent, ThemeColor::Rgb(12, 180, 160));
        if custom {
            overrides = overrides.with_color(ThemeRole::Chart, ThemeColor::Rgb(230, 160, 40));
        }
        session = session.with_theme_overrides(overrides);
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 70), &pin)
                .unwrap();
            let mut bars = 0;
            let mut traces = 0;
            let mut heading = false;
            for y in 0..frame.surface.size().height {
                let mut row = String::new();
                for x in 0..frame.surface.size().width {
                    let cell = frame.surface.cell(Point::new(x, y)).unwrap();
                    if let CellContent::Grapheme { text, .. } = cell.content() {
                        row.push_str(text);
                        if text
                            .chars()
                            .any(|c| ('\u{2800}'..='\u{28ff}').contains(&c) || c == '█')
                        {
                            assert_eq!(
                                cell.style().foreground,
                                if custom { chart } else { accent }
                            );
                            if text.contains('█') {
                                bars += 1;
                            } else {
                                traces += 1;
                            }
                        }
                    } else {
                        row.push(' ');
                    }
                }
                if let Some(x) = row.chars().position(|c| c == 'C')
                    && row.contains("Chart heading")
                {
                    assert_eq!(
                        frame
                            .surface
                            .cell(Point::new(x as u16, y))
                            .unwrap()
                            .style()
                            .foreground,
                        accent
                    );
                    heading = true;
                }
            }
            assert!(heading && bars > 0 && traces > 0);
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
}
// 네 계열의 범례·점은 각각 지정 색상을 받고 흑백에서도 번호와 원래 수치가 유지된다.
#[test]
fn named_chart_colors_and_mono_legends_preserve_original_values() {
    use crate::{OutputPreferences, Theme, ThemeColor, ThemeOverrides, ThemeRole};
    let source =
        "```linechart\nheight: 6\nFirst: 30 30\nSecond: 20 20\nThird: 10 10\nFourth: 0 0\n```";
    let roles = [
        ThemeRole::Chart,
        ThemeRole::Chart2,
        ThemeRole::Chart3,
        ThemeRole::Chart4,
    ];
    let mut overrides = ThemeOverrides::default();
    for (i, role) in roles.into_iter().enumerate() {
        overrides = overrides.with_color(role, ThemeColor::Rgb(40 + i as u8 * 40, 100, 180));
    }
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
        .with_theme_overrides(overrides);
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source.to_owned()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for theme in [Theme::Default, Theme::Mono, Theme::Light] {
        session = session.with_theme(theme);
        for width in [80, 24, 12, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            for name in ["[1]First", "[2]Second", "[3]Third", "[4]Fourth"] {
                assert!(text.contains(name), "{text}");
            }
            assert!(text.contains("Fourth:00"), "{text}");
            if theme != Theme::Mono {
                let mut colors = [false; 4];
                for y in 0..frame.surface.size().height {
                    for x in 0..frame.surface.size().width {
                        let color = frame
                            .surface
                            .cell(Point::new(x, y))
                            .unwrap()
                            .style()
                            .foreground;
                        for (i, found) in colors.iter_mut().enumerate() {
                            *found |= color
                                == Color::Rgb {
                                    red: 40 + i as u8 * 40,
                                    green: 100,
                                    blue: 180,
                                };
                        }
                    }
                }
                assert!(colors.into_iter().all(|found| found));
            }
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
}
