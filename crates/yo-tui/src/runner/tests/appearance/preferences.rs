use super::support::*;

// 출력 설정은 실제 접기·개행에 적용되고 테마 선택 순서와 무관하게 유지된다.
// 최대 u16 설정도 overflow 없이 전체 로그를 보이며 원문과 실패 사유는 소실되지 않는다.
#[test]
fn output_preferences_control_real_frames_without_losing_source() {
    use std::num::NonZeroU16;

    use yo_core::{ActivityOutcome, Failure};

    use crate::{OutputPreferences, Theme};
    for head in [0, 4, u16::MAX] {
        let preferences = OutputPreferences::default()
            .with_tool_head_rows(head)
            .with_max_body_width(NonZeroU16::new(24));
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(preferences)
            .with_theme(Theme::Light);
        let source = (1..=20)
            .map(|n| format!("LOG {n:02} abcdefghijklmnopqrstuvwxyz"))
            .collect::<Vec<_>>()
            .join("\n");
        let pin = session.appearance_pin();
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(source),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("VISIBLE FAILURE")),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(80, 60), &pin).unwrap();
        let visible = visible_rows(&frame.surface);
        assert_eq!(visible.contains("LOG 02"), head >= 4, "{visible}");
        assert_eq!(
            visible.contains("rows hidden"),
            head != u16::MAX,
            "{visible}"
        );
        assert!(visible.contains("LOG 20"), "{visible}");
        assert!(visible.contains("VISIBLE FAILURE"), "{visible}");
        assert!(
            !visible.contains("abcdefghijklmnopqrstuvwxyz"),
            "configured width must wrap the body"
        );
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("LOG 02"));
        assert!(!plain.contains("rows hidden"));
    }
}

// diff 접기 설정은 도구 로그 설정과 독립적이며 원래 경로·변경 줄·실패 footer를 보존한다.
#[test]
fn diff_head_preference_controls_changed_lines_independently() {
    use yo_core::ActivityOutcome;

    use crate::OutputPreferences;
    for head in [0, u16::MAX] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_diff_head_rows(head),
            );
        let pin = session.appearance_pin();
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::FileChange,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(format!(
                    "update: file.rs\n{}",
                    (1..=24)
                        .map(|n| format!("+VALUE {n:02}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
        let visible = visible_rows(&frame.surface);
        assert_eq!(visible.contains("VALUE 02"), head != 0, "{visible}");
        assert!(visible.contains("VALUE 24"), "{visible}");
        assert!(
            session
                .session_output()
                .unwrap()
                .unwrap()
                .contains("VALUE 02")
        );
    }
}

// 1열 선호는 최소 2열로 정규화하여 정상 터미널에서 한글 본문·코드가 렌더 실패를 만들지 않는다.
#[test]
fn one_column_preference_keeps_wide_unicode_renderable() {
    use std::num::NonZeroU16;

    use crate::OutputPreferences;
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(
            OutputPreferences::default().with_max_body_width(NonZeroU16::new(1)),
        );
    let pin = session.appearance_pin();
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
            update: ActivityUpdate::TextSnapshot("한글\n\n```rust\n한글\n```".into()),
        })
        .unwrap();
    let frame = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
    let visible = visible_rows(&frame.surface);
    assert!(
        visible.contains('한') && visible.contains('글'),
        "{visible}"
    );
}

// 사용자 팔레트는 실제 코드·diff·메시지 셀에 적용되고 기존 강조 및 설정을 보존한다.
// 테마 왕복과 제한된 색상·무색 터미널에서 같은 역할이 안전한 색상으로 해석된다.
#[test]
fn semantic_theme_overrides_reach_frames_and_survive_theme_switches() {
    use crate::{Theme, ThemeColor, ThemeOverrides, ThemeRole};
    for (capability, expected) in [
        (
            ColorCapability::TrueColor,
            Color::Rgb {
                red: 95,
                green: 135,
                blue: 175,
            },
        ),
        (ColorCapability::Limited, Color::Indexed(67)),
        (ColorCapability::Unknown, Color::Default),
    ] {
        let overrides = ThemeOverrides::default()
            .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(95, 135, 175))
            .with_color(
                ThemeRole::DiffAddedBackground,
                ThemeColor::Rgb(95, 135, 175),
            )
            .with_color(ThemeRole::UserBackground, ThemeColor::Rgb(95, 135, 175));
        let mut session = TuiSession::new(capability, MotionPreference::Reduced)
            .with_theme_overrides(overrides)
            .with_theme(Theme::Light)
            .with_theme(Theme::Mono)
            .with_theme(Theme::Default);
        let pin = session.appearance_pin();
        let state = session.parts_mut().state;
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from("USER BAND"),
                },
            ))
            .unwrap();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::AgentMessage,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    "```rust\nlet answer = 42;\n```\n\n```diff\n+added\n```".into(),
                ),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(60, 28), &pin).unwrap();
        let mut checked = 0;
        for y in 0..28 {
            let row = (0..60)
                .map(
                    |x| match frame.surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Grapheme { text, .. } => text.to_string(),
                        _ => " ".to_owned(),
                    },
                )
                .collect::<String>();
            if ["USER BAND", "let answer", "+added"]
                .iter()
                .any(|text| row.contains(text))
            {
                let cell = (2..60).map(|x| frame.surface.cell(Point::new(x,y)).unwrap())
                    .find(|cell| matches!(cell.content(), CellContent::Grapheme { text, .. } if !text.trim().is_empty())).unwrap();
                assert_eq!(cell.style().background, expected, "{row}");
                if row.contains("USER BAND") {
                    assert!(cell.style().attributes.contains(Attributes::BOLD));
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 3);
        let mono = session.with_theme(Theme::Mono);
        assert_eq!(
            mono.appearance_pin()
                .snapshot()
                .styles()
                .transcript
                .markdown
                .code
                .background,
            Color::Default
        );
        let restored = mono
            .with_theme(Theme::Default)
            .with_theme_overrides(ThemeOverrides::default());
        let baseline = TuiSession::new(capability, MotionPreference::Reduced);
        assert_eq!(
            restored.appearance_pin().snapshot().styles(),
            baseline.appearance_pin().snapshot().styles()
        );
    }
}

// 이미지 표시 설정은 테마를 바꾼 실제 프레임에도 적용되며 원본 PNG와 출력 기록을 보존한다.
#[test]
fn image_preferences_reach_frames_and_keep_original_payload() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};

    use crate::{OutputPreferences, Theme};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(120, 60)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let bytes = encoded.into_inner();
    let source = format!(
        "![Original](data:image/png;base64,{})",
        STANDARD.encode(&bytes)
    );
    for (visible, limit, expected) in [(true, 1, 1), (true, 8, 8), (true, 65, 64), (false, 8, 0)] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_images(visible)
                    .with_image_max_width(NonZeroU16::new(limit).unwrap()),
            )
            .with_theme(Theme::Light);
        let pin = session.appearance_pin();
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
                update: ActivityUpdate::TextSnapshot(source.clone()),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(100, 60), &pin).unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("Original"), "{text}");
        assert_eq!(text.contains("Image display disabled"), !visible, "{text}");
        assert!(!text.contains("base64"), "{text}");
        if visible {
            assert_eq!(frame.surface.rasters.len(), 1);
            let raster = &frame.surface.rasters[0];
            assert_eq!(raster.area.size.width, expected);
            assert_eq!(&*raster.png, bytes.as_slice());
        } else {
            assert!(frame.surface.rasters.is_empty());
            assert!(!text.contains('▀'));
        }
        assert!(session.session_output().unwrap().unwrap().contains(&source));
    }
}

// 코드 여백 설정은 실제 프레임의 코드·diff와 테마 변경에 적용되고 원문은 그대로 유지된다.
#[test]
fn code_padding_controls_code_and_diff_frames_after_resize() {
    use yo_core::{ActivityDocument, ActivityOutcome};

    use crate::{DocumentRenderer, OutputPreferences, Theme, ToolRenderer};
    for (kind, source) in [
        (
            ActivityKind::AgentMessage,
            "```rust\nMARKER\n```".to_owned(),
        ),
        (
            ActivityKind::FileChange,
            "update: file.rs\n+MARKER\n".to_owned(),
        ),
        (ActivityKind::ToolCall, "custom tool source".to_owned()),
        (
            ActivityKind::ModelWork,
            ActivityDocument {
                title: "Document".to_owned(),
                markdown: "original document source".to_owned(),
            }
            .to_snapshot()
            .unwrap(),
        ),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
        if kind == ActivityKind::ToolCall {
            session = session.with_tool_renderer(Some(ToolRenderer::new(|_| {
                Some("```rust\nMARKER\n```".to_owned())
            })));
        } else if kind == ActivityKind::ModelWork {
            session = session.with_document_renderer(Some(DocumentRenderer::new(|_| {
                Some("```rust\nMARKER\n```".to_owned())
            })));
        }
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(source.to_owned()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let original = session.session_output().unwrap();
        for padding in [0, 1, 4, 8, 9, u16::MAX, 1] {
            session = session.with_output_preferences(
                OutputPreferences::default()
                    .with_code_padding(padding)
                    .with_diff_head_rows(u16::MAX),
            );
            for theme in [Theme::Default, Theme::Light, Theme::Mono] {
                session = session.with_theme(theme);
                for width in [80, 12, 80] {
                    let pin = session.appearance_pin();
                    let frame = session
                        .parts_mut()
                        .state
                        .prepare_frame(Size::new(width, 60), &pin)
                        .unwrap();
                    let text = visible_rows(&frame.surface);
                    assert!(
                        text.split_whitespace()
                            .collect::<String>()
                            .contains("MARKER"),
                        "{padding}/{width}: {text}"
                    );
                    if width == 80 {
                        let row = text.lines().find(|line| line.contains("MARKER")).unwrap();
                        let expected = 2
                            + usize::from(padding.min(8))
                            + usize::from(kind == ActivityKind::FileChange);
                        assert_eq!(row.find("MARKER"), Some(expected), "{text}");
                    }
                    assert_eq!(session.session_output().unwrap(), original);
                }
            }
        }
    }
}

// 코드 글자색을 기본색·RGB로 바꿔도 이미지 색 깊이와 셀은 변하지 않는다.
#[test]
fn code_color_overrides_do_not_change_image_color_capability() {
    use std::io::Cursor;

    use ::image::{ImageFormat, Rgb, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::ActivityOutcome;

    use crate::{Theme, ThemeColor, ThemeOverrides, ThemeRole};
    let image = RgbImage::from_fn(4, 2, |x, y| Rgb([(x * 60) as u8, (y * 180) as u8, 100]));
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).unwrap();
    let source = format!(
        "```text\nCODE\n```\n\n![Image](data:image/png;base64,{})",
        STANDARD.encode(png.into_inner())
    );
    for capability in [
        ColorCapability::TrueColor,
        ColorCapability::Limited,
        ColorCapability::Unknown,
    ] {
        for theme in [Theme::Default, Theme::Light, Theme::Mono] {
            let mut session = TuiSession::with_session_info(
                GlyphProfile::Rich,
                TuiSessionInfo::default(),
                capability,
                MotionPreference::Reduced,
            )
            .with_theme(theme);
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
                    update: ActivityUpdate::TextSnapshot(source.clone()),
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: ActivityOutcome::Completed,
                })
                .unwrap();
            let original = session.session_output().unwrap();
            let pin = session.appearance_pin();
            let baseline = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(40, 30), &pin)
                .unwrap();
            let image_cells = |frame: &Surface| {
                frame.rasters.first().map(|raster| {
                    let area = raster.area;
                    (0..area.size.width)
                        .map(|x| {
                            frame
                                .cell(Point::new(area.origin.x + x, area.origin.y))
                                .unwrap()
                                .clone()
                        })
                        .collect::<Vec<_>>()
                })
            };
            let expected = image_cells(&baseline.surface);
            for color in [ThemeColor::Terminal, ThemeColor::Rgb(1, 2, 3)] {
                session = session.with_theme_overrides(
                    ThemeOverrides::default().with_color(ThemeRole::CodeText, color),
                );
                for width in [40, 12, 40] {
                    let pin = session.appearance_pin();
                    let frame = session
                        .parts_mut()
                        .state
                        .prepare_frame(Size::new(width, 30), &pin)
                        .unwrap();
                    let actual = image_cells(&frame.surface);
                    assert_eq!(
                        actual, expected,
                        "{capability:?}/{theme:?}/{color:?}/{width}"
                    );
                    let text = visible_rows(&frame.surface);
                    let code_row =
                        text.lines().position(|line| line.contains("CODE")).unwrap() as u16;
                    let code_color = frame
                        .surface
                        .cell(Point::new(3, code_row))
                        .unwrap()
                        .style()
                        .foreground;
                    if color == ThemeColor::Terminal
                        || capability == ColorCapability::Unknown
                        || theme == Theme::Mono
                    {
                        assert_eq!(code_color, Color::Default);
                    } else if capability == ColorCapability::TrueColor {
                        assert_eq!(
                            code_color,
                            Color::Rgb {
                                red: 1,
                                green: 2,
                                blue: 3
                            }
                        );
                    }
                    assert_eq!(text.contains('▀'), expected.is_some());
                    assert_eq!(session.session_output().unwrap(), original);
                }
            }
        }
    }
}
