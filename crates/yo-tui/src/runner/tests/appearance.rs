use yo_core::{
    ActivityKind, ActivityUpdate, AgentCommand, AgentEvent, TranscriptRecord, UserInput,
};

use super::{activity, turn};
use crate::{
    PresentationMode, TuiSessionInfo,
    appearance::{
        AppearanceCandidate, AppearanceState, ColorCapability, GlyphProfile, MotionPreference,
    },
    html::HtmlSurface,
    prompt::{PromptGlyphs, PromptStyles},
    runner::{session::TuiSession, state::TuiState},
    shell::{AgentShellStyles, ShellChromeStyles},
    surface::{Attributes, CellContent, Color, FrameDiff, Point, Size, Style, Surface},
    terminal::{TerminalOp, TerminalOps},
    transcript::{MarkdownStyles, TranscriptActivityStyles, TranscriptStyles},
};

const FRAME_SIZE: Size = Size::new(20, 11);

fn conversation() -> TuiState {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();
    let assistant = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: assistant,
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: assistant,
            update: ActivityUpdate::TextSnapshot("answer".to_owned()),
        })
        .unwrap();
    state
}

fn populate_session(session: &mut TuiSession) {
    let state = session.parts_mut().state;
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();
}

fn marker(surface: &Surface, row: u16) -> (&str, u16, Style) {
    let cell = surface.cell(Point::new(0, row)).unwrap();
    let CellContent::Grapheme { text, width } = cell.content() else {
        panic!("the transcript row must begin with a marker");
    };
    (text, width.get(), cell.style())
}

fn grapheme_at(surface: &Surface, point: Point) -> &str {
    let CellContent::Grapheme { text, .. } = surface.cell(point).unwrap().content() else {
        panic!("the selected cell must contain a grapheme");
    };
    text
}

fn visible_rows(surface: &Surface) -> String {
    let size = surface.size();
    let mut rows = (0..size.height)
        .map(|y| {
            (0..size.width)
                .filter_map(
                    |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank | CellContent::Continuation { .. } => Some(' '),
                        CellContent::Grapheme { text, .. } => text.chars().next(),
                    },
                )
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows.join("\n")
}

// 한 frame이 측정 뒤 appearance 교체를 만나도 pinned Rich snapshot으로 끝까지 paint한다.
#[test]
fn frame_pins_one_snapshot_across_measure_and_paint() {
    let state = conversation();
    let mut appearance = AppearanceState::default();
    let rich = appearance.pin();

    let frame = state
        .prepare_frame_with_measure_hook(FRAME_SIZE, &rich, || {
            appearance
                .commit(AppearanceCandidate::for_profile(GlyphProfile::Ascii))
                .unwrap();
        })
        .unwrap();

    assert_eq!(frame.appearance_revision, rich.revision());
    assert_eq!(marker(&frame.surface, 0).0, "❯");
    assert_eq!(marker(&frame.surface, 2).0, "•");

    let ascii = appearance.pin();
    let next = state.prepare_frame(FRAME_SIZE, &ascii).unwrap();
    assert_eq!(ascii.revision().get(), rich.revision().get() + 1);
    assert_eq!(marker(&next.surface, 0).0, ">");
    assert_eq!(marker(&next.surface, 2).0, "*");
}

// Rich와 ASCII profile 모두 marker 폭과 무관하게 사용자·assistant 본문을 같은 열에 둔다.
#[test]
fn rich_and_ascii_profiles_keep_body_columns_stable() {
    let state = conversation();
    let rich = state
        .prepare_frame(FRAME_SIZE, &AppearanceState::default().pin())
        .unwrap();
    let ascii_state =
        AppearanceState::new(AppearanceCandidate::for_profile(GlyphProfile::Ascii)).unwrap();
    let ascii = state.prepare_frame(FRAME_SIZE, &ascii_state.pin()).unwrap();

    assert_eq!(marker(&rich.surface, 0).1, 1);
    assert_eq!(marker(&rich.surface, 2).1, 1);
    assert_eq!(marker(&ascii.surface, 0).1, 1);
    assert_eq!(marker(&ascii.surface, 2).1, 1);
    for surface in [&rich.surface, &ascii.surface] {
        assert_eq!(grapheme_at(surface, Point::new(2, 0)), "q");
        assert_eq!(grapheme_at(surface, Point::new(2, 2)), "a");
    }
}

// 기본 Rich와 ASCII profile은 충분한 높이에서 각각의 prompt marker/rule glyph를 쓰고,
// terminal-default 본문·bold marker·dim rule 역할을 resolved Surface에 그대로 남긴다.
#[test]
fn default_profiles_resolve_prompt_glyphs_and_visual_roles() {
    let state = TuiState::new();
    let size = Size::new(10, 9);
    let rich = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    let ascii_state =
        AppearanceState::new(AppearanceCandidate::for_profile(GlyphProfile::Ascii)).unwrap();
    let ascii = state.prepare_frame(size, &ascii_state.pin()).unwrap();
    let body = Style::default();
    let marker_style = Style::new(Color::Default, Color::Default, Attributes::BOLD);
    let rule_style = Style::new(Color::Default, Color::Default, Attributes::DIM);

    assert_eq!(marker(&rich.surface, 4), ("─", 1, rule_style));
    assert_eq!(marker(&rich.surface, 5), ("›", 1, marker_style));
    assert_eq!(marker(&rich.surface, 6), ("─", 1, rule_style));
    assert_eq!(marker(&ascii.surface, 4), ("-", 1, rule_style));
    assert_eq!(marker(&ascii.surface, 5), (">", 1, marker_style));
    assert_eq!(marker(&ascii.surface, 6), ("-", 1, rule_style));
    assert_eq!(rich.surface.cell(Point::new(2, 5)).unwrap().style(), body);
    assert_eq!(ascii.surface.cell(Point::new(2, 5)).unwrap().style(), body);
}

// public session 생성자로 선택한 ASCII snapshot은 실제 준비 frame과 종료용 plain output에
// 함께 쓰여 host가 선택한 profile의 일관성과 출력 경계를 지킨다.
#[test]
fn public_ascii_session_keeps_frame_and_output_consistent() {
    let mut session = TuiSession::with_glyph_profile(
        GlyphProfile::Ascii,
        ColorCapability::Unknown,
        MotionPreference::Standard,
    );
    *session.parts_mut().state = conversation();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(FRAME_SIZE, &pin)
        .unwrap();
    let output = session.session_output().unwrap().unwrap();

    assert_eq!(
        visible_rows(&frame.surface),
        "> question\n\n* answer\n\n\n\n--------------------\n>\n--------------------\n\nEnter send    inline"
    );
    assert_eq!(output, "> question\n\n* answer\n");
}

// 명시적인 Unknown·Standard host 선택을 받은 public session은 Rich snapshot을 실제
// 준비 frame과 plain output에 함께 사용해 기본 glyph profile의 일관성을 지킨다.
#[test]
fn public_rich_session_keeps_frame_and_output_consistent() {
    let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    *session.parts_mut().state = conversation();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(FRAME_SIZE, &pin)
        .unwrap();

    assert_eq!(
        visible_rows(&frame.surface),
        "❯ question\n\n• answer\n\n\n\n────────────────────\n›\n────────────────────\n\nEnter send    inline"
    );
    assert_eq!(
        session.session_output().unwrap().unwrap(),
        "❯ question\n\n• answer\n"
    );
}

// host가 제공한 실제 metadata와 active lifecycle은 같은 frame의 작업 행·metrics·mode로
// 전달되며, compatibility 기본값을 backend처럼 꾸며내지 않는다.
#[test]
fn session_projects_host_metadata_active_work_and_presentation_mode() {
    let mut session = TuiSession::with_session_info(
        GlyphProfile::Rich,
        TuiSessionInfo::new("codex", "~/projects/yo"),
        ColorCapability::TrueColor,
        MotionPreference::Standard,
    );
    session.set_presentation_mode(PresentationMode::Fullscreen);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    let pin = session.appearance_pin();

    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(88, 12), &pin)
        .unwrap();
    let rows = visible_rows(&frame.surface);

    assert!(rows.contains("⠋ Working"));
    assert!(rows.contains("codex · ~/projects/yo"));
    assert!(rows.ends_with("fullscreen"));
}

// 실제 Chat 작업 marker가 보일 때만 PreparedFrame이 16ms motion demand를 보고하고,
// 같은 marker 구간의 서로 다른 elapsed에서도 동일한 frame geometry를 유지한다.
#[test]
fn visible_activity_marker_alone_demands_timed_motion() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    let appearance = AppearanceState::new(AppearanceCandidate::for_profile_with_host_preferences(
        GlyphProfile::Rich,
        ColorCapability::TrueColor,
        MotionPreference::Standard,
    ))
    .unwrap()
    .pin();

    let first = state
        .prepare_frame_at(Size::new(48, 12), &appearance, Duration::from_millis(500))
        .unwrap();
    let second = state
        .prepare_frame_at(Size::new(48, 12), &appearance, Duration::from_millis(516))
        .unwrap();

    assert_eq!(
        first.motion_demand.unwrap().period(),
        Duration::from_millis(16)
    );
    assert_eq!(second.motion_demand, first.motion_demand);
    assert_eq!(first.surface.size(), second.surface.size());
    assert_eq!(visible_rows(&first.surface), visible_rows(&second.surface));
    assert!(visible_rows(&first.surface).contains("⠴ Working"));
    assert_ne!(first.surface, second.surface);
}

// 실제 public session 경계에서 Reduced를 선택하면 작업 표시는 그대로 보이지만,
// frame이 16ms motion demand를 만들지 않아 host가 접근성 선택을 실행할 수 있다.
#[test]
fn public_reduced_motion_session_keeps_activity_static() {
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    let pin = session.appearance_pin();

    let frame = session
        .parts_mut()
        .state
        .prepare_frame_at(Size::new(48, 12), &pin, Duration::from_secs(9))
        .unwrap();

    assert!(visible_rows(&frame.surface).contains("⠋ Working"));
    assert_eq!(frame.motion_demand, None);
}

// 작업 중이어도 marker를 생략하는 좁은 fallback이나 작업 행 자체가 없는 낮은 화면은
// 보이지 않는 애니메이션을 위해 timer를 요구하지 않는다.
#[test]
fn hidden_activity_marker_does_not_demand_timed_motion() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    let appearance = AppearanceState::default().pin();

    let narrow = state
        .prepare_frame_at(Size::new(6, 12), &appearance, Duration::ZERO)
        .unwrap();
    let short = state
        .prepare_frame_at(Size::new(48, 2), &appearance, Duration::ZERO)
        .unwrap();

    assert!(visible_rows(&narrow.surface).contains("Esc/^C"));
    assert_eq!(narrow.motion_demand, None);
    assert_eq!(short.motion_demand, None);
}

// 한 TuiSession의 profile 교체는 다른 세션의 snapshot과 revision에 전파되지 않는다.
#[test]
fn appearance_replacement_is_isolated_per_session() {
    let mut first = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    let mut second = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    populate_session(&mut first);
    populate_session(&mut second);
    let second_before = second.appearance_pin();

    first.select_glyph_profile(GlyphProfile::Ascii).unwrap();
    let first_pin = first.appearance_pin();
    let first_frame = first
        .parts_mut()
        .state
        .prepare_frame(FRAME_SIZE, &first_pin)
        .unwrap();
    let second_current_frame = second
        .parts_mut()
        .state
        .prepare_frame(FRAME_SIZE, &second_before)
        .unwrap();
    let second_next = second.appearance_pin();

    assert_eq!(marker(&first_frame.surface, 0).0, ">");
    assert_eq!(first.session_output().unwrap().unwrap(), "> question\n");
    assert_eq!(marker(&second_current_frame.surface, 0).0, "❯");
    assert_eq!(second.session_output().unwrap().unwrap(), "❯ question\n");
    assert_eq!(second_next, second_before);
    assert_eq!(
        second_before.snapshot().transcript_config().user_marker(),
        "❯"
    );
}

// completed Surface의 marker 폭과 style은 terminal op와 HTML projection에 그대로 전달된다.
#[test]
fn terminal_and_html_project_the_same_completed_appearance_surface() {
    let state = conversation();
    let marker_style = Style::new(Color::Indexed(45), Color::Indexed(17), Attributes::BOLD);
    let default = Style::default();
    let styles = AgentShellStyles {
        transcript: TranscriptStyles {
            background: default,
            user_marker: marker_style,
            user_body: default,
            assistant_marker: default,
            assistant_body: default,
            activity: TranscriptActivityStyles::plain(Style::default()),
            markdown: MarkdownStyles::plain(Style::default()),
        },
        prompt: PromptStyles {
            body: default,
            marker: default,
            rule: default,
            glyphs: PromptGlyphs::ascii(),
        },
        chrome: ShellChromeStyles {
            activity: crate::appearance::ActivityStyles {
                marker: default,
                muted: default,
                trail: default,
                peak: default,
            },
            metrics: default,
            mode: default,
            key_hint: default,
        },
        overlay: crate::overlay::SelectionPanelAppearance {
            styles: crate::overlay::SelectionPanelStyles {
                activity: crate::appearance::ActivityStyles {
                    marker: default,
                    muted: default,
                    trail: default,
                    peak: default,
                },
                background: default,
                frame: default,
                title: default,
                key_hint: default,
                hint: default,
                label: default,
                detail: default,
                selected: default,
                disabled: default,
            },
            glyphs: crate::overlay::SelectionPanelGlyphs::ascii(),
        },
    };
    let appearance = AppearanceState::new(
        AppearanceCandidate::for_profile(GlyphProfile::Ascii).with_styles_for_test(styles),
    )
    .unwrap();
    let frame = state.prepare_frame(FRAME_SIZE, &appearance.pin()).unwrap();
    let diff = FrameDiff::complete(FRAME_SIZE, &frame.surface);
    let operations = TerminalOps::from_diff(&diff);
    let html = HtmlSurface::render(&frame.surface);

    assert_eq!(marker(&frame.surface, 0), (">", 1, marker_style));
    assert!(operations.as_slice().windows(2).any(|pair| {
        matches!(
            pair,
            [
                TerminalOp::SetStyle(style),
                TerminalOp::WriteGrapheme { text: ">", width }
            ] if *style == marker_style && width.get() == 1
        )
    }));
    assert!(html.contains(
        "data-column=\"0\" data-width=\"1\" data-fg=\"indexed-45\" \
         data-bg=\"indexed-17\" data-attrs=\"bold\""
    ));
    assert!(html.contains("<span class=\"yo-glyph\""));
    assert!(html.contains("\">"));
}
use std::time::Duration;

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

// 호스트 도구 본문은 실제 코드·표·이미지 프레임으로 표시되며 실패 상태·원문·표시 설정을 보존한다.
#[test]
fn tool_renderer_uses_rich_layout_without_replacing_status_or_source() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityOutcome, Failure};

    use crate::{OutputPreferences, Theme, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(32, 16)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let markdown = format!(
        "```rust\nlet value = 42;\n```\n\n| Key | Value |\n| --- | --- |\n| 상태 | ready |\n\n![Tool image](data:image/png;base64,{})",
        STANDARD.encode(&png)
    );
    for show_images in [true, false] {
        let renderer = ToolRenderer::new({
            let markdown = markdown.clone();
            move |input| {
                assert_eq!(input.kind, ActivityKind::ToolCall);
                assert_eq!(input.source, "docs.search\noriginal payload");
                assert_eq!(input.columns.get(), 30);
                Some(markdown.clone())
            }
        });
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(renderer))
            .with_output_preferences(
                OutputPreferences::default()
                    .with_max_body_width(NonZeroU16::new(30))
                    .with_tool_head_rows(u16::MAX)
                    .with_images(show_images)
                    .with_image_max_width(NonZeroU16::new(4).unwrap()),
            )
            .with_theme(Theme::Light);
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
                update: ActivityUpdate::TextSnapshot("docs.search\noriginal payload".to_owned()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("VISIBLE FAILURE")),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(60, 50), &pin).unwrap();
        let text = visible_rows(&frame.surface);
        for expected in [
            "Tool failed",
            "VISIBLE FAILURE",
            "let value = 42;",
            "ready",
            "Tool image",
        ] {
            assert!(text.contains(expected), "{expected}: {text}");
        }
        assert!(!text.contains("original payload"));
        assert_eq!(text.contains("Image display disabled"), !show_images);
        if show_images {
            assert_eq!(frame.surface.rasters.len(), 1);
            assert_eq!(frame.surface.rasters[0].area.size.width, 4);
            assert_eq!(&*frame.surface.rasters[0].png, png.as_slice());
        } else {
            assert!(frame.surface.rasters.is_empty());
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("original payload"));
        assert!(plain.contains("VISIBLE FAILURE"));
        assert!(!plain.contains("let value = 42;"));
        session = session.with_tool_renderer(None);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 50), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("original payload"));
        assert!(frame.surface.rasters.is_empty());
    }
}

// 적용 거절·높이 초과는 원문으로 돌아가며 새 렌더러 설치는 이미 캐시된 레이아웃을 갱신한다.
#[test]
fn tool_renderer_fallback_and_replacement_invalidate_layout() {
    use crate::ToolRenderer;

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("original payload".to_owned()),
        })
        .unwrap();
    for (renderer, expected) in [
        (
            ToolRenderer::new(|_| Some("replacement one".to_owned())),
            "replacement one",
        ),
        (
            ToolRenderer::new(|_| Some("replacement two".to_owned())),
            "replacement two",
        ),
        (ToolRenderer::new(|_| None), "original payload"),
        (
            ToolRenderer::new(|_| Some("x\n\n".repeat(32769))),
            "original payload",
        ),
    ] {
        session = session.with_tool_renderer(Some(renderer));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(expected), "{text}");
        assert!(text.contains("Tool result"), "{text}");
    }
}

// 도구 확장 콜백은 일반 답변·추론·파일 diff를 가로채지 않는다.
#[test]
fn tool_renderer_does_not_intercept_other_activity_kinds() {
    use crate::ToolRenderer;

    for kind in [
        ActivityKind::AgentMessage,
        ActivityKind::ModelWork,
        ActivityKind::FileChange,
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(|_| {
                panic!("non-tool must not reach renderer")
            })));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("literal payload".to_owned()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 20), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("literal payload"));
    }
}

// 접기 구간과 겹친 native raster는 제거하고 아래쪽에 완전히 남은 raster는 셀과 함께 이동한다.
#[test]
fn tool_renderer_folding_keeps_native_images_aligned() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};

    use crate::{OutputPreferences, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(8, 4)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let image = format!(
        "![Image](data:image/png;base64,{})",
        STANDARD.encode(encoded.into_inner())
    );
    for at_end in [true, false] {
        let rows = "row\n\n".repeat(20);
        let markdown = if at_end {
            format!("{rows}{image}")
        } else {
            format!("{image}\n\n{rows}")
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(move |_| Some(markdown.clone()))))
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_image_max_width(NonZeroU16::new(8).unwrap()),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("payload".to_owned()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let full = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 70), &pin)
            .unwrap();
        assert_eq!(full.surface.rasters.len(), 1);
        let original_y = full.surface.rasters[0].area.origin.y;
        session = session.with_output_preferences(
            OutputPreferences::default()
                .with_tool_head_rows(0)
                .with_image_max_width(NonZeroU16::new(8).unwrap()),
        );
        let pin = session.appearance_pin();
        let folded = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 70), &pin)
            .unwrap();
        assert!(visible_rows(&folded.surface).contains("rows hidden"));
        if at_end {
            assert_eq!(folded.surface.rasters.len(), 1);
            assert!(folded.surface.rasters[0].area.origin.y < original_y);
            assert_eq!(folded.surface.rasters[0].png, full.surface.rasters[0].png);
        } else {
            assert!(folded.surface.rasters.is_empty());
        }
    }
}

// 명시적 도구 출력 profile의 PNG·인수는 기본 렌더러에 도달하고 사용자 콜백은 원본 JSON을 받는다.
#[test]
fn structured_tool_output_reaches_native_images_and_custom_renderer() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(16, 8)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let data = STANDARD.encode(&png);
    let mut output = ToolOutput {
        tool: "screenshot".to_owned(),
        server: Some("browser".to_owned()),
        arguments: Some("original args".into()),
        result: None,
        content_items: None,
        error: None,
        plain_text: "browser.screenshot\nReadable result".to_owned(),
    };
    output.result = Some(
        format!(r#"{{"content":[{{"type":"image","mimeType":"image/png","data":"{data}"}}]}}"#)
            .parse()
            .unwrap(),
    );
    for (show, dynamic, resource) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (true, false, true),
        (false, false, true),
    ] {
        let mut output = output.clone();
        if dynamic {
            output.result = None;
            output.content_items = Some(
                format!(r#"[{{"type":"inputImage","imageUrl":"data:image/png;base64,{data}"}}]"#)
                    .parse()
                    .unwrap(),
            );
        }
        if resource {
            output.result = Some(
                json!({"content":[{"type":"resource","resource":{"uri":"resource://image","mimeType":"image/png","blob":data}}]}),
            );
        }
        let wire = output.to_snapshot().unwrap();
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_images(show)
                    .with_image_max_width(NonZeroU16::new(4).unwrap()),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(wire.clone()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 50), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("browser.screenshot"), "{text}");
        assert!(text.contains("original args"), "{text}");
        assert!(!text.contains(&data));
        assert_eq!(text.contains("Image display disabled"), !show);
        if show {
            assert_eq!(frame.surface.rasters.len(), 1);
            assert_eq!(&*frame.surface.rasters[0].png, png.as_slice());
            assert_eq!(frame.surface.rasters[0].area.size.width, 4);
        } else {
            assert!(frame.surface.rasters.is_empty());
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("Readable result"), "{plain}");
        assert!(!plain.contains(ToolOutput::SCHEMA), "{plain}");
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            assert_eq!(input.source, expected.plain_text);
            Some("**Custom typed result**".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 50), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom typed result"));
    }
    // A literal text block containing fences and image syntax must not become media.
    let literal = format!("```\n![literal](data:image/png;base64,{data})\n```");
    let result = output.result.as_mut().unwrap();
    result["content"][0]["type"] = "text".into();
    result["content"][0]["text"] = literal.into();
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(70, 50), &pin)
        .unwrap();
    assert!(frame.surface.rasters.is_empty());
    output.result = Some(r#"{"content":[],"structuredContent":{"ok":true}}"#.parse().unwrap());
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(70, 50), &pin)
        .unwrap();
    let text = visible_rows(&frame.surface);
    assert!(text.contains("(empty content)"), "{text}");
    assert!(text.contains("Structured result"), "{text}");
}

// 경고 안내는 완료 상태로 바뀌지 않고 warning palette·개행을 유지하며 원문 내보내기도 읽을 수 있다.
#[test]
fn retry_and_limit_notices_keep_warning_style_after_delivery() {
    use yo_core::{ActivityNotice, ActivityOutcome, Failure, NoticeLevel};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};

    for notice in [ActivityNotice {
        title: "Retry announced".to_owned(),
        message: "temporary disconnect\nCodex will retry.".to_owned(),
        level: NoticeLevel::Warning,
    }, ActivityNotice {
        title: "Response limit reached".to_owned(),
        message: "The response stopped before completion.\nPartial answer text is retained. Unfinished tool calls were not executed.".to_owned(),
        level: NoticeLevel::Warning,
    }, ActivityNotice {
        title: "Model rerouted".to_owned(),
        message: "Codex reported a model change.\nFrom: requested\nTo: actual\nReason: highRiskCyberActivity".to_owned(),
        level: NoticeLevel::Warning,
    }] {
    for outcome in [
        ActivityOutcome::Completed,
        ActivityOutcome::Interrupted,
        ActivityOutcome::Failed(Failure::new("notice delivery failed")),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Warning, ThemeColor::Rgb(210, 170, 90)),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(notice.to_snapshot().unwrap()),
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: outcome.clone(),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(40, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(&notice.title), "{text}");
        assert!(text.contains(&notice.message.chars().take(20).collect::<String>()), "{text}");
        assert!(!text.contains("Model work completed"), "{text}");
        let title = frame.surface.cell(Point::new(2, 0)).unwrap();
        assert_eq!(
            title.style().foreground,
            Color::Rgb {
                red: 210,
                green: 170,
                blue: 90
            }
        );
        assert!(title.style().attributes.contains(Attributes::BOLD));
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains(&notice.title));
        assert!(!plain.contains(ActivityNotice::SCHEMA));
        if matches!(outcome, ActivityOutcome::Interrupted) {
            assert!(plain.contains("Interrupted"));
        }
        if matches!(outcome, ActivityOutcome::Failed(_)) {
            assert!(plain.contains("notice delivery failed"));
        }
    }
    }
}

// 시작 경고는 가짜 Turn 없이 표시하고 경고 색상·문자 원문을 보존하며 Working을 시작하지 않는다.
#[test]
fn session_notice_renders_before_any_turn_with_custom_warning_style() {
    use yo_core::{ActivityNotice, NoticeLevel};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_theme_overrides(
            ThemeOverrides::default().with_color(ThemeRole::Warning, ThemeColor::Rgb(210, 170, 90)),
        );
    session
        .parts_mut()
        .state
        .observe_notice(ActivityNotice {
            title: "Configuration warning".to_owned(),
            message: "Literal **setting**\nFile: config.toml".to_owned(),
            level: NoticeLevel::Warning,
        })
        .unwrap();
    assert!(!session.parts_mut().state.turn_active());
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(40, 20), &pin)
        .unwrap();
    let text = visible_rows(&frame.surface);
    assert!(text.contains("Configuration warning"), "{text}");
    assert!(text.contains("Literal **setting**"), "{text}");
    assert!(text.contains("File: config.toml"), "{text}");
    assert!(!text.contains("Working"), "{text}");
    assert!(!text.contains("Thinking"), "{text}");
    assert!(frame.motion_demand.is_none());
    let title = frame.surface.cell(Point::new(2, 0)).unwrap();
    assert_eq!(
        title.style().foreground,
        Color::Rgb {
            red: 210,
            green: 170,
            blue: 90
        }
    );
    assert!(title.style().attributes.contains(Attributes::BOLD));
    let plain = session.session_output().unwrap().unwrap();
    assert!(plain.contains("Literal **setting**"));
    assert!(!plain.contains(ActivityNotice::SCHEMA));
}

// 압축 안내는 같은 항목의 시작 문구를 완료 snapshot으로 교체하고 accent 설정과 원문을 유지한다.
#[test]
fn context_compaction_updates_one_styled_notice_without_thinking_or_raw_json() {
    use yo_core::{ActivityNotice, ActivityOutcome, NoticeLevel};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_theme_overrides(
            ThemeOverrides::default().with_color(ThemeRole::Accent, ThemeColor::Rgb(90, 170, 210)),
        );
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    for title in ["Compacting context", "Context compacted"] {
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivityNotice {
                        title: title.to_owned(),
                        message: "Conversation context".to_owned(),
                        level: NoticeLevel::Info,
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        if title == "Context compacted" {
            session
                .parts_mut()
                .state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: ActivityOutcome::Completed,
                })
                .unwrap();
        }
        assert_eq!(session.parts_mut().state.transcript().items().len(), 1);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(40, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(title), "{text}");
        assert!(!text.contains("Thinking"), "{text}");
        assert!(!text.contains("Model work completed"), "{text}");
        assert!(!text.contains(ActivityNotice::SCHEMA), "{text}");
        let heading = frame.surface.cell(Point::new(2, 0)).unwrap();
        assert_eq!(
            heading.style().foreground,
            Color::Rgb {
                red: 90,
                green: 170,
                blue: 210
            }
        );
        assert!(heading.style().attributes.contains(Attributes::BOLD));
    }
    let plain = session.session_output().unwrap().unwrap();
    assert!(plain.contains("Context compacted"));
    assert!(!plain.contains("Compacting context"));
}

// 압축·분기 요약은 접어도 원문과 실패 footer를 유지하며 Ctrl+O로 코드 강조까지 복원한다.
#[test]
fn activity_summary_folds_markdown_preserving_source_and_outcome() {
    use std::time::Duration;

    use yo_core::{ActivityOutcome, ActivitySummary, Failure, SummaryKind};

    use super::key;
    use crate::{
        OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole,
        input::event::{KeyCode, KeyModifiers},
    };

    for kind in [SummaryKind::Compaction, SummaryKind::Branch] {
        let summary = ActivitySummary {
            kind,
            summary: format!(
                "## Decisions\n\n{}\n\n```rust\nuse std::fmt;\n```",
                (0..20)
                    .map(|n| format!("- retained decision {n:02}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            tokens_before: (kind == SummaryKind::Compaction).then_some(12000),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(1))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Accent, ThemeColor::Rgb(90, 170, 210)),
            );
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(summary.to_snapshot().unwrap()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("delivery stopped")),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for line in summary.summary.lines() {
            assert!(original.contains(line), "{original}");
        }
        assert!(!original.contains(ActivitySummary::SCHEMA));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 55), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("Ctrl+O expand"), "{text}");
        assert!(!text.contains("retained decision 10"), "{text}");
        assert!(text.contains("delivery stopped"), "{text}");
        assert_eq!(
            text.contains("12000 tokens"),
            kind == SummaryKind::Compaction
        );
        let heading = frame.surface.cell(Point::new(2, 0)).unwrap();
        assert_eq!(
            heading.style().foreground,
            Color::Rgb {
                red: 90,
                green: 170,
                blue: 210
            }
        );
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 55), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("retained decision 10"), "{text}");
        assert!(text.contains("use std::fmt;"), "{text}");
        assert!(!text.contains("Ctrl+O expand"), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 빈 요약은 안내를 남기고 좁은 폭에서도 schema를 노출하지 않으며 Markdown 이미지를 첨부로 실행하지
// 않는다.
#[test]
fn activity_summary_empty_and_narrow_image_fallbacks_are_visible() {
    use std::{io::Cursor, time::Duration};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityOutcome, ActivitySummary, SummaryKind};

    use super::key;
    use crate::input::event::{KeyCode, KeyModifiers};
    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(120, 60)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let source_image = format!(
        "![untrusted](data:image/png;base64,{})",
        STANDARD.encode(encoded.into_inner())
    );
    for source in ["", source_image.as_str()] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivitySummary {
                        kind: SummaryKind::Branch,
                        summary: source.to_owned(),
                        tokens_before: None,
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let pin = session.appearance_pin();
        for width in [4, 24, 60] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            assert!(frame.surface.rasters.is_empty());
            let text = visible_rows(&frame.surface);
            assert!(!text.contains(ActivitySummary::SCHEMA), "{text}");
            if source.is_empty() && width == 60 {
                assert!(text.contains("No summary text was provided."), "{text}");
            }
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(
            plain
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
                .contains(source)
        );
    }
}

// 공개 요약만 숨기고 계획·경고·압축 요약과 실패 footer는 유지하며 설정을 바꾸면 원문을 복원한다.
#[test]
fn public_reasoning_visibility_preserves_other_activity_and_plain_export() {
    use yo_core::{
        ActivityNotice, ActivityOutcome, ActivitySummary, Failure, NoticeLevel, SummaryKind,
    };

    use crate::OutputPreferences;

    let populate = |session: &mut TuiSession| {
        for (id, text) in [
            (
                1,
                ActivitySummary {
                    kind: SummaryKind::Reasoning,
                    summary: "**Visible reasoning body**".to_owned(),
                    tokens_before: None,
                }
                .to_snapshot()
                .unwrap(),
            ),
            (
                2,
                ActivitySummary {
                    kind: SummaryKind::Compaction,
                    summary: "Keep compaction visible".to_owned(),
                    tokens_before: Some(900),
                }
                .to_snapshot()
                .unwrap(),
            ),
            (
                3,
                ActivityNotice {
                    title: "Keep warning visible".to_owned(),
                    message: "Configuration issue".to_owned(),
                    level: NoticeLevel::Warning,
                }
                .to_snapshot()
                .unwrap(),
            ),
            (4, "Plan\n- Keep plan visible".to_owned()),
        ] {
            let state = session.parts_mut().state;
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: activity(id),
                    kind: ActivityKind::ModelWork,
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(id),
                    update: ActivityUpdate::TextSnapshot(text),
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(id),
                    outcome: if id == 1 {
                        ActivityOutcome::Failed(Failure::new("summary delivery stopped"))
                    } else {
                        ActivityOutcome::Completed
                    },
                })
                .unwrap();
        }
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_reasoning(false));
    populate(&mut session);
    let source = session.session_output().unwrap().unwrap();
    assert!(source.contains("**Visible reasoning body**"));
    for visible in [false, true, false] {
        session =
            session.with_output_preferences(OutputPreferences::default().with_reasoning(visible));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 45), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Visible reasoning body"), visible, "{text}");
        assert_eq!(text.contains("Summary hidden"), !visible, "{text}");
        for retained in [
            "Keep compaction visible",
            "Keep warning visible",
            "Keep plan visible",
            "summary delivery stopped",
        ] {
            assert!(text.contains(retained), "{text}");
        }
        assert!(!text.contains(ActivitySummary::SCHEMA), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 실제 다이어그램 프레임은 코드 팔레트를 사용하고 표시 설정·폭을 바꿔도 내보낼 Mermaid 원문을
// 유지한다.
#[test]
fn mermaid_frame_preferences_preserve_source_and_palette() {
    use yo_core::ActivityOutcome;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    let source = "```mermaid\ngraph TD; A[Build] --> B[Test]\n```";
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_theme_overrides(
            ThemeOverrides::default()
                .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(25, 35, 45)),
        );
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source.to_owned()),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("graph TD; A[Build] --> B[Test]"));
    for (enabled, width) in [(true, 80), (false, 80), (true, 16), (true, 80)] {
        session =
            session.with_output_preferences(OutputPreferences::default().with_diagrams(enabled));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        if enabled && width == 80 {
            assert!(!text.contains("graph TD"), "{text}");
            assert!(text.contains("Build") && text.contains("Test"), "{text}");
            let row = (0..40).find(|y| {
                (0..width).any(|x| matches!(frame.surface.cell(Point::new(x,*y)).unwrap().content(), CellContent::Grapheme {text,..} if text.as_ref() == "│"))
            }).expect("diagram connector row");
            assert_eq!(
                frame
                    .surface
                    .cell(Point::new(3, row))
                    .unwrap()
                    .style()
                    .background,
                Color::Rgb {
                    red: 25,
                    green: 35,
                    blue: 45
                }
            );
        } else if !enabled {
            assert!(text.contains("graph TD"), "{text}");
        }
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 계획은 상태별 팔레트·본문 들여쓰기를 유지하고 실패·완료가 단계 상태나 문자 원문을 바꾸지 않는다.
#[test]
fn activity_plan_styles_wraps_and_preserves_reported_progress() {
    use yo_core::{ActivityOutcome, ActivityPlan, Failure, PlanStep, PlanStepStatus};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};
    let plan = ActivityPlan {
        explanation: Some("**literal explanation**".to_owned()),
        steps: vec![
            PlanStep {
                text: "Completed".to_owned(),
                status: PlanStepStatus::Completed,
            },
            PlanStep {
                text: "ABCDEFGHIJKLMNO".to_owned(),
                status: PlanStepStatus::InProgress,
            },
            PlanStep {
                text: "Pending".to_owned(),
                status: PlanStepStatus::Pending,
            },
        ],
    };
    for outcome in [
        ActivityOutcome::Completed,
        ActivityOutcome::Failed(Failure::new("plan delivery failed")),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Success, ThemeColor::Rgb(80, 180, 90))
                    .with_color(ThemeRole::Accent, ThemeColor::Rgb(90, 140, 220)),
            );
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(plan.to_snapshot().unwrap()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: outcome.clone(),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(20, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("[x] Completed"), "{text}");
        assert!(text.contains("[>] ABCDEFGHIJKLMN"), "{text}");
        assert!(text.contains("      O"), "{text}");
        assert!(text.contains("[ ] Pending"), "{text}");
        for (needle, color, bold) in [
            (
                "[x]",
                Color::Rgb {
                    red: 80,
                    green: 180,
                    blue: 90,
                },
                false,
            ),
            (
                "[>]",
                Color::Rgb {
                    red: 90,
                    green: 140,
                    blue: 220,
                },
                true,
            ),
        ] {
            let y = (0..40)
                .find(|y| {
                    let row = (0..20)
                        .filter_map(|x| {
                            match frame.surface.cell(Point::new(x, *y)).unwrap().content() {
                                CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                                _ => None,
                            }
                        })
                        .collect::<String>();
                    row.contains(needle)
                })
                .unwrap();
            let style = frame.surface.cell(Point::new(2, y)).unwrap().style();
            assert_eq!(style.foreground, color);
            if bold {
                assert!(style.attributes.contains(Attributes::BOLD));
            }
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("1/3 completed"), "{plain}");
        assert!(plain.contains("**literal explanation**"), "{plain}");
        assert!(plain.contains("ABCDEFGHIJKLMNO"), "{plain}");
        assert!(!plain.contains(ActivityPlan::SCHEMA));
        if matches!(outcome, ActivityOutcome::Failed(_)) {
            assert!(
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .contains("plandeliveryfailed"),
                "{text}"
            );
        }
    }
}

// 빈 계획은 0/0 진행률을 만들지 않고 명시적 안내를 표시한다.
#[test]
fn activity_plan_empty_steps_have_an_explicit_fallback() {
    use yo_core::{ActivityOutcome, ActivityPlan};
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                ActivityPlan {
                    explanation: None,
                    steps: Vec::new(),
                }
                .to_snapshot()
                .unwrap(),
            ),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(40, 15), &pin)
        .unwrap();
    let text = visible_rows(&frame.surface);
    assert!(text.contains("No steps provided."), "{text}");
    assert!(!text.contains("0/0"), "{text}");
    assert!(!text.contains("Thinking"), "{text}");
    assert!(!text.contains(ActivityPlan::SCHEMA), "{text}");
}

// 제안 계획은 Markdown 본문을 교체해 폭별로 다시 배치하고 중단 후에도 제목·원문·footer를 유지한다.
#[test]
fn proposed_plan_reflows_final_document_without_draft_or_json() {
    use std::time::Duration;

    use yo_core::{ActivityDocument, ActivityOutcome};

    use super::key;
    use crate::{
        OutputPreferences,
        input::event::{KeyCode, KeyModifiers},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_reasoning(false));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    let final_source = "## Final approach\n\n- Keep the source\n\n```rust\nuse std::fmt;\n```\n\n| Check | Value |\n| --- | --- |\n| width | reflow |";
    for source in ["## Draft only", final_source] {
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivityDocument {
                        title: "Proposed plan".to_owned(),
                        markdown: source.to_owned(),
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
    }
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextDelta(String::new()),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Interrupted,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for line in final_source.lines() {
        assert!(original.contains(line), "{original}");
    }
    assert!(!original.contains("Draft only"));
    assert!(!original.contains(ActivityDocument::SCHEMA));
    let pin = session.appearance_pin();
    for width in [60, 24, 60] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 55), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("Proposed plan"), "{text}");
        assert!(text.contains("Final approach"), "{text}");
        assert!(text.contains("use std::fmt;"), "{text}");
        assert!(text.contains("reflow"), "{text}");
        assert!(text.contains("Interrupted"), "{text}");
        assert!(!text.contains("Draft only"), "{text}");
        assert!(!text.contains("Summary hidden"), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 터미널 입력의 fence·이미지·제어 문자는 실행되지 않고 폭 변경 뒤에도 코드 원문으로 남는다.
#[test]
fn terminal_input_document_keeps_markdown_and_controls_literal() {
    use std::{io::Cursor, time::Duration};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityDocument, ActivityOutcome};

    use super::key;
    use crate::input::event::{KeyCode, KeyModifiers};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(8, 4)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let input = format!(
        "```\n![literal](data:image/png;base64,{})\n# literal heading\n\u{3}\u{1b}[31m\n끝까지 보이는 입력",
        STANDARD.encode(encoded.into_inner())
    );
    let source = format!("````text\nProcess: test-7\nCommand: cargo test\nInput:\n{input}\n````");
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                ActivityDocument {
                    title: "Terminal input sent".to_owned(),
                    markdown: source,
                }
                .to_snapshot()
                .unwrap(),
            ),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("![literal](data:image/png;base64,"));
    assert!(original.contains("# literal heading"));
    assert!(!original.contains(ActivityDocument::SCHEMA));
    let pin = session.appearance_pin();
    for width in [60, 24, 60] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 65), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(frame.surface.rasters.is_empty());
        assert!(text.contains("Terminal input sent"), "{text}");
        assert!(text.contains("```"), "{text}");
        assert!(text.contains("# literal heading"), "{text}");
        assert!(text.contains("^C^[[31m"), "{text}");
        assert!(
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .contains("끝까지보이는입력"),
            "{text}"
        );
        assert!(!text.contains(ActivityDocument::SCHEMA), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 보고된 턴 시간은 좁은 폭에서 다시 줄바꿈해도 값·출처·원문 내보내기를 유지한다.
#[test]
fn reported_turn_duration_reflows_without_losing_precision() {
    use yo_core::{ActivityNotice, ActivityOutcome, NoticeLevel};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                ActivityNotice {
                    title: "Turn completed".to_owned(),
                    message: "Duration: 1m 2.345s (62345 ms, reported by Codex)".to_owned(),
                    level: NoticeLevel::Info,
                }
                .to_snapshot()
                .unwrap(),
            ),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let source = session.session_output().unwrap().unwrap();
    assert!(source.contains("62345 ms, reported by Codex"));
    let pin = session.appearance_pin();
    for width in [60, 24, 60] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 35), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        let joined = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(
            joined.contains("Duration:1m2.345s(62345ms,reportedbyCodex)"),
            "{text}"
        );
        assert!(text.contains("Turn completed"), "{text}");
        assert!(!text.contains(ActivityNotice::SCHEMA), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 파일 읽기·리소스의 Rust 원문은 사용자 지정 코드 색상을 쓰며 오류·비파일 출력은 일반 텍스트로
// 남는다.
#[test]
fn file_tool_output_highlights_only_known_successful_source() {
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (tool, resource, failed, expected_code) in [
        ("read", false, false, true),
        ("read_file", false, false, true),
        ("lookup", true, false, true),
        ("lookup", false, false, false),
        ("read", false, true, false),
    ] {
        let source = "fn main() { println!(\"hello\"); }\n```\n![literal](ignored.png)";
        let mut output = ToolOutput {
            tool: tool.to_owned(), server: Some("files".to_owned()),
            arguments: Some(r#"{"path":"src/main.rs","offset":7,"limit":3}"#.parse().unwrap()),
            result: Some(if resource {
                r#"{"content":[{"type":"resource","resource":{"uri":"file:///src/main.rs","text":""}}],"isError":false}"#
            } else {
                r#"{"content":[{"type":"text","text":""}],"isError":false}"#
            }.parse().unwrap()),
            content_items: None, error: None,
            plain_text: format!("files.{tool}\nsrc/main.rs\n{source}"),
        };
        let result = output.result.as_mut().unwrap();
        result["isError"] = failed.into();
        if resource {
            result["content"][0]["resource"]["text"] = source.into();
        } else {
            result["content"][0]["text"] = source.into();
        }
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(17, 211, 119)),
            );
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
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for line in source.lines() {
            assert!(original.contains(line), "{original}");
        }
        let pin = session.appearance_pin();
        for width in [70, 24, 70] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 65), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            assert!(
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .contains("src/main.rs"),
                "{text}"
            );
            assert!(text.contains("![literal]"), "{text}");
            assert!(text.contains("```"), "{text}");
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains("fn main()"))
                .unwrap();
            let x = row.find("fn main()").unwrap();
            let color = frame
                .surface
                .cell(Point::new(x as u16, y as u16))
                .unwrap()
                .style()
                .foreground;
            assert_eq!(
                color
                    == Color::Rgb {
                        red: 17,
                        green: 211,
                        blue: 119
                    },
                expected_code,
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        session = session.with_tool_renderer(Some(ToolRenderer::new(|_| {
            Some("Custom file view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom file view"));
    }
}

// 쓰기 제안은 인수 JSON과 분리해 코드 색상을 적용하며 빈 파일·오류·잘못된 인수도 원문과 구분을
// 유지한다.
#[test]
fn proposed_file_content_is_highlighted_without_claiming_execution() {
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (content, failed) in [
        (Some("fn main() {}\n```\n![literal](file.png)"), false),
        (Some("fn main() {}"), true),
        (Some(""), false),
        (None, false),
    ] {
        let mut output = ToolOutput {
            tool: "write_file".to_owned(),
            server: None,
            arguments: Some(
                r#"{"path":"src/main.rs","content":"","create":true}"#
                    .parse()
                    .unwrap(),
            ),
            result: None,
            content_items: None,
            error: None,
            plain_text: format!("write_file\nsrc/main.rs\n{}", content.unwrap_or("7")),
        };
        output.arguments.as_mut().unwrap()["content"] =
            content.map_or_else(|| 7.into(), Into::into);
        if failed {
            output.error = Some("permission denied".into());
        }
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(19, 212, 117)),
            );
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
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [70, 24, 70] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 65), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(
                joined.contains("Proposedfilecontent"),
                content.is_some(),
                "{text}"
            );
            assert!(!text.contains("Tool completed"), "{text}");
            assert!(frame.surface.rasters.is_empty());
            if let Some(content) = content {
                assert!(!text.contains("\"content\""), "{text}");
                if content.is_empty() {
                    assert!(text.contains("(empty file)"), "{text}");
                } else {
                    assert_eq!(text.matches("fn main()").count(), 1, "{text}");
                    let (y, row) = text
                        .lines()
                        .enumerate()
                        .find(|(_, line)| line.contains("fn main()"))
                        .unwrap();
                    let x = row.find("fn main()").unwrap();
                    assert_eq!(
                        frame
                            .surface
                            .cell(Point::new(x as u16, y as u16))
                            .unwrap()
                            .style()
                            .foreground,
                        Color::Rgb {
                            red: 19,
                            green: 212,
                            blue: 117
                        }
                    );
                }
            } else {
                assert!(text.contains("\"content\": 7"), "{text}");
            }
            if failed {
                assert!(text.contains("permission denied"), "{text}");
            }
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        if !failed {
            use yo_core::ActivityOutcome;
            session
                .parts_mut()
                .state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: ActivityOutcome::Completed,
                })
                .unwrap();
            assert!(
                session
                    .session_output()
                    .unwrap()
                    .unwrap()
                    .contains("Tool call prepared")
            );
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(70, 60), &pin)
                .unwrap();
            assert!(visible_rows(&frame.surface).contains("Tool call prepared"));
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom write view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom write view"));
    }
}

// 여러 파일 읽기는 원래 순서·실제 줄 범위·개별 오류를 보존하며 손상·확장된 결과는 통째로 원문
// 표시한다.
#[test]
fn batch_read_panels_preserve_ranges_errors_and_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    let good = json!({"results":[
        {"path":"src/main.rs","status":"ok","start":2,"end":3,"total":8,"next_offset":4,"content":"fn main() {\n}\n"},
        {"path":"missing.txt","status":"error","error":"unavailable"},
        {"path":"empty.txt","status":"ok","start":0,"end":0,"total":0,"content":""}
    ]});
    let mut unknown = good.clone();
    unknown["results"][0]["unknown"] = "retained".into();
    let mut invalid_range = good.clone();
    invalid_range["results"][0]["next_offset"] = 5.into();
    let mut invalid_content = good.clone();
    invalid_content["results"][0]["content"] = "only one line".into();
    let encoded = good.to_string();
    let at_limit = format!("{}{}", " ".repeat(256 * 1024 - encoded.len()), encoded);
    let over_limit = format!(" {at_limit}");
    let mut eight = good.clone();
    for _ in 0..5 {
        eight["results"]
            .as_array_mut()
            .unwrap()
            .push(good["results"][1].clone());
    }
    let mut nine = eight.clone();
    nine["results"]
        .as_array_mut()
        .unwrap()
        .push(good["results"][1].clone());
    for (source, rich) in [
        (good.to_string(), true),
        (at_limit, true),
        (over_limit, false),
        (eight.to_string(), true),
        (nine.to_string(), false),
        (unknown.to_string(), false),
        (invalid_range.to_string(), false),
        (invalid_content.to_string(), false),
        (
            "{\"results\":[\n[yo: tool output truncated]".to_owned(),
            false,
        ),
    ] {
        let output = ToolOutput {
            tool: "read_files".to_owned(),
            server: None,
            arguments: Some(
                json!({"files":[{"path":"src/main.rs"},{"path":"missing.txt"},{"path":"empty.txt"}]}),
            ),
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: format!("read_files\n{source}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(18, 213, 118)),
            );
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let exported = original
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        let content = source
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(exported.contains(&content));
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 110), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("Lines2–3of8"), rich, "{text}");
            if rich {
                assert!(joined.contains("Continuereadingatline4"), "{text}");
                assert!(joined.contains("Readfailed·missing.txt"), "{text}");
                assert!(text.contains("unavailable"), "{text}");
                assert!(text.contains("(empty file)"), "{text}");
                let (y, row) = text
                    .lines()
                    .enumerate()
                    .find(|(_, line)| line.contains("fn main()"))
                    .unwrap();
                let x = row.find("fn main()").unwrap();
                assert_eq!(
                    frame
                        .surface
                        .cell(Point::new(x as u16, y as u16))
                        .unwrap()
                        .style()
                        .foreground,
                    Color::Rgb {
                        red: 18,
                        green: 213,
                        blue: 118
                    }
                );
                assert!(text.find("fn main()").unwrap() < text.find("unavailable").unwrap());
                assert!(text.find("unavailable").unwrap() < text.find("(empty file)").unwrap());
            } else {
                assert!(joined.contains("\"results\":"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom batch view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom batch view"));
    }
}

// 교체 제안의 삭제·추가 색상과 개행 차이를 유지하고 실제 적용 개수·실패·원본 인수를 별도로
// 보존한다.
#[test]
fn edit_proposals_keep_diff_styles_and_reported_results_separate() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (valid, failed) in [(true, false), (true, true), (false, false)] {
        let mut args = json!({"path":"src/main.rs","edits":[
            {"oldText":"let old = 1;","newText":"let new = 2;\n"},
            {"oldText":"remove me\n","newText":""}
        ]});
        if !valid {
            args["edits"][0]["extra"] = "retained".into();
        }
        let source = r#"{"path":"src/main.rs","status":"ok","replacements":2}"#;
        let output = ToolOutput {
            tool: "edit_file".to_owned(),
            server: None,
            arguments: Some(args),
            result: Some(json!({"content":[{"type":"text","text":source}],"isError":failed})),
            content_items: None,
            error: failed.then(|| "permission denied".into()),
            plain_text: format!("edit_file\nlet old = 1;\nlet new = 2;\n{source}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::DiffAdded, ThemeColor::Rgb(20, 214, 120))
                    .with_color(ThemeRole::DiffRemoved, ThemeColor::Rgb(215, 21, 120)),
            );
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
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        assert!(original.contains(source));
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("Proposedreplacements"), valid, "{text}");
            assert_eq!(joined.contains("Appliedreplacements:2"), !failed, "{text}");
            if valid {
                assert!(!text.contains("\"oldText\""), "{text}");
                assert!(joined.contains("Oldtexthasnotrailingnewline"), "{text}");
                assert!(!joined.contains("Newtexthasnotrailingnewline"), "{text}");
                assert!(text.contains("-remove me"), "{text}");
                for (needle, color) in [
                    (
                        "-let old",
                        Color::Rgb {
                            red: 215,
                            green: 21,
                            blue: 120,
                        },
                    ),
                    (
                        "+let new",
                        Color::Rgb {
                            red: 20,
                            green: 214,
                            blue: 120,
                        },
                    ),
                ] {
                    let (y, row) = text
                        .lines()
                        .enumerate()
                        .find(|(_, line)| line.contains(needle))
                        .unwrap();
                    let x = row.find(needle).unwrap();
                    assert_eq!(
                        frame
                            .surface
                            .cell(Point::new(x as u16, y as u16))
                            .unwrap()
                            .style()
                            .foreground,
                        color
                    );
                }
            } else {
                assert!(text.contains("\"extra\""), "{text}");
                assert!(joined.contains("retained"), "{text}");
            }
            assert!(!text.contains("@@ -"), "{text}");
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom edit view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom edit view"));
    }
}

// 교체 수·본문 바이트 상한과 첫 초과 값에서 제안 표시 여부를 구분하고 단일 교체 형식도 지원한다.
#[test]
fn edit_proposal_limits_preserve_literal_fallbacks() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    let replacement = json!({"oldText":"old\n","newText":"new\n"});
    for (args, marker, rich) in [
        (
            json!({"path":"file.rs","edits":vec![replacement.clone();256]}),
            "Replacement 256",
            true,
        ),
        (
            json!({"path":"file.rs","edits":vec![replacement;257]}),
            "Replacement 257",
            false,
        ),
        (
            json!({"path":"file.rs","oldText":"x".repeat(256*1024),"newText":""}),
            "Old text has no trailing newline",
            true,
        ),
        (
            json!({"path":"file.rs","oldText":"x".repeat(256*1024+1),"newText":""}),
            "Old text has no trailing newline",
            false,
        ),
    ] {
        let output = ToolOutput {
            tool: "edit_file".to_owned(),
            server: None,
            arguments: Some(args.clone()),
            result: None,
            content_items: None,
            error: None,
            plain_text: args.to_string(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
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
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains(marker), rich, "expected {marker}: {text}");
        assert!(frame.surface.rasters.is_empty());
    }
}

// 성공한 내장 수정·쓰기 결과만 개수로 요약하고 잘못되거나 확장된 상태는 원문으로 남긴다.
#[test]
fn mutation_result_summaries_require_exact_success_payloads() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    let mut at_limit = json!({"path":"","status":"ok","bytes":0});
    at_limit["path"] = "p".repeat(16 * 1024 - at_limit.to_string().len()).into();
    let mut over_limit = at_limit.clone();
    over_limit["path"] = format!("{}p", at_limit["path"].as_str().unwrap()).into();
    for (tool, result, expected) in [
        ("write_file", at_limit, Some("Written bytes: 0")),
        ("write_file", over_limit, None),
        (
            "edit_file",
            json!({"path":"file.rs","status":"ok","replacements":2}),
            Some("Applied replacements: 2"),
        ),
        (
            "write_file",
            json!({"path":"file.rs","status":"ok","bytes":0}),
            Some("Written bytes: 0"),
        ),
        (
            "edit_file",
            json!({"path":"file.rs","status":"error","replacements":2}),
            None,
        ),
        (
            "edit_file",
            json!({"path":"file.rs","status":"ok","replacements":-1}),
            None,
        ),
        (
            "edit_file",
            json!({"path":"file.rs","status":"ok","replacements":2,"extra":"keep"}),
            None,
        ),
    ] {
        let source = result.to_string();
        let output = ToolOutput {
            tool: tool.to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: source.clone(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(90, 35), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        if let Some(expected) = expected {
            assert!(text.contains(expected), "{text}");
        } else {
            assert!(!text.contains("Applied replacements"), "{text}");
            assert!(text.contains("\"status\""), "{text}");
        }
        assert!(session.session_output().unwrap().unwrap().contains(&source));
    }
}

// 명령·실제 스트림·종료 상태를 분리하되 모호하거나 잘린 출력은 원문으로 보존하고 콜백 우선권을
// 유지한다.
#[test]
fn shell_command_and_streams_preserve_literal_boundaries() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (source, parsed) in [
        (
            "status: 0\nstdout:\nhello\n```\n![literal](file.png)\n\u{1b}[31m\nstderr:\nwarning",
            true,
        ),
        ("status: signal\nstdout:\n\nstderr:\n", true),
        ("status: 1\nstdout:\nfailure\nstderr:\nerror", true),
        (
            "status: 0\nstdout:\nfake\nstderr:\ninside stdout\nstderr:\nactual stderr",
            false,
        ),
        (
            "status: 0\nstdout:\npartial\nstderr:\n[yo: tool output truncated]",
            false,
        ),
        ("status: unknown\nstdout:\nx\nstderr:\ny", false),
    ] {
        let command = "if true; then\n  printf '%s\\n' hello\nfi";
        let output = ToolOutput {
            tool: "run_command".to_owned(),
            server: None,
            arguments: Some(json!({"command":command,"timeout":30})),
            result: Some(json!({"content":[{"type":"text","text":source}],"isError":true})),
            content_items: None,
            error: None,
            plain_text: format!("{command}\n{source}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_shell_tail_rows(u16::MAX),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(22, 216, 122)),
            );
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(text.contains("Command"), "{text}");
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains("if true; then"))
                .unwrap();
            let x = row.find("if true; then").unwrap();
            assert_eq!(
                frame
                    .surface
                    .cell(Point::new(x as u16, y as u16))
                    .unwrap()
                    .style()
                    .foreground,
                Color::Rgb {
                    red: 22,
                    green: 216,
                    blue: 122
                }
            );
            assert!(!text.contains("\"command\""), "{text}");
            assert!(text.contains("\"timeout\": 30"), "{text}");
            assert_eq!(joined.contains("Exitstatus:"), parsed, "{text}");
            if source.contains("![literal]") {
                assert!(text.contains("![literal]"), "{text}");
                assert!(text.contains("^[[31m"), "{text}");
            }
            if source.contains("signal") {
                assert!(text.contains("(no stdout)"), "{text}");
                assert!(text.contains("(no stderr)"), "{text}");
            }
            if !parsed {
                assert!(text.contains("status:"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom shell view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom shell view"));
    }
}

// 여러 명령 도구의 합산 출력은 native stdout/stderr로 오해하지 않으며 같은 메타데이터·색상·원문을
// 보존한다.
#[test]
fn command_panels_preserve_aggregate_output_and_customization_across_tools() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for tool in ["commandExecution", "bash", "powershell"] {
        let command = if tool == "powershell" {
            "if ($true) {\n  Write-Output test\n}"
        } else {
            "if true; then\n  echo test\nfi"
        };
        let command_start = command.lines().next().unwrap();
        let source = "status: 0\nstdout:\n![literal](file.png)\nstderr:\ncombined";
        let output = ToolOutput {
            tool: tool.to_owned(),
            server: None,
            arguments: Some(json!({"command":command, "cwd":"/workspace"})),
            result: Some(
                json!({"content":[{"type":"text","text":source}], "exitCode":101,"durationMs":1240,"status":"failed"}),
            ),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_shell_tail_rows(u16::MAX),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(22, 216, 122)),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            for expected in [
                "Command",
                "Exitstatus:101",
                "Duration:1240ms",
                "Status:failed",
                "![literal](file.png)",
                "status:0",
            ] {
                assert!(joined.contains(expected), "{text}");
            }
            assert!(!joined.contains("Exitstatus:0"), "{text}");
            assert!(!text.contains("\"command\""), "{text}");
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, row)| row.contains(command_start))
                .unwrap();
            let x = row.find(command_start).unwrap();
            // 내장 문법이 없는 PowerShell은 원문을 보존하고 지원되는 Bash만 키워드 색상을 검증한다.
            if tool != "powershell" {
                assert_eq!(
                    frame
                        .surface
                        .cell(Point::new(x as u16, y as u16))
                        .unwrap()
                        .style()
                        .foreground,
                    Color::Rgb {
                        red: 22,
                        green: 216,
                        blue: 122
                    }
                );
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom command".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom command"));
    }
}

// 디렉터리 표시는 실제 잘림 메타데이터를 사용하고 같은 이름의 파일·빈 결과·원문·콜백을 보존한다.
#[test]
fn directory_listing_uses_observed_truncation_and_literal_names() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (source, truncated, rich, partial) in [
        (
            "src/\nREADME.md\n![literal](file.png)\n",
            Some(false),
            true,
            false,
        ),
        ("", Some(false), true, false),
        (
            "src/\n\n[yo: tool output truncated]",
            Some(true),
            true,
            true,
        ),
        ("\n[yo: tool output truncated]", Some(true), true, true),
        ("src/\n", Some(true), true, true),
        ("[yo: tool output truncated]\n", Some(false), true, false),
        ("unterminated", Some(false), false, false),
        ("unterminated", Some(true), false, false),
        ("src/\n", None, false, false),
        ("bad\u{1b}name\n", Some(false), false, false),
    ] {
        let mut result = json!({"content":[{"type":"text","text":source}]});
        if let Some(truncated) = truncated {
            result["truncated"] = truncated.into();
        }
        let output = ToolOutput {
            tool: "list_files".to_owned(),
            server: None,
            arguments: Some(json!({"path":"."})),
            result: Some(result),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 65), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(
                joined.contains("entriesshown") || joined.contains("entryshown"),
                rich,
                "{text}"
            );
            assert_eq!(joined.contains("Partiallisting"), partial, "{text}");
            assert!(!text.contains("\"truncated\""), "{text}");
            assert_eq!(
                text.contains("Output truncated"),
                truncated == Some(true) && !rich,
                "{text}"
            );
            if source.starts_with("[yo:") {
                assert!(joined.contains("file[yo:tooloutputtruncated]"), "{text}");
            }
            if source.starts_with("src/") && rich {
                assert!(joined.contains("dirsrc/"), "{text}");
                assert!(joined.contains("1directory"), "{text}");
            }
            if source.contains("![literal]") {
                assert!(text.contains("![literal]"), "{text}");
            }
            if source.is_empty() {
                assert!(joined.contains("Noentrieswerereturned."), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom directory view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom directory view"));
    }
}

// 파일 검색은 패턴·결과·보고된 제한을 구분하되 잘못된 메타데이터와 원본은 유지한다.
// 좁은 폭에서도 파일 이름을 Markdown으로 실행하지 않고 사용자 renderer가 원본을 받는다.
#[test]
fn file_search_preserves_literals_limits_and_custom_renderer_across_resize() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    let source = "src/main.rs\n![literal](file.png)";
    for (details, partial) in [
        (json!({"resultLimitReached": 1}), true),
        (
            json!({"resultLimitReached": 2, "truncation": {
                "content": source, "truncated": true, "truncatedBy": "lines",
                "totalLines": 5, "totalBytes": 500, "outputLines": 2, "outputBytes": source.len(),
                "lastLinePartial": false, "firstLineExceedsLimit": false, "maxLines": 2, "maxBytes": 1024
            }}),
            true,
        ),
        (json!({"resultLimitReached": u64::MAX}), true),
        (json!({"resultLimitReached": 0}), false),
        (json!({"resultLimitReached": -1}), false),
        (json!({"resultLimitReached": 1.5}), false),
        (json!({"resultLimitReached": "2"}), false),
        (json!({"resultLimitReached": 2, "future": true}), false),
        (json!({"resultLimitReached": 2, "truncation": {}}), false),
    ] {
        let output = ToolOutput {
            tool: "find".to_owned(),
            server: None,
            arguments: Some(json!({"pattern": "**/*.rs", "path": ".", "limit": 2})),
            result: Some(
                json!({"content": [{"type": "text", "text": source}], "details": details}),
            ),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 90), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text.split_whitespace().collect::<String>();
            assert!(joined.contains("Filesearchpattern"), "{text}");
            assert!(joined.contains("**/*.rs"), "{text}");
            assert!(joined.contains("Matchingpaths"), "{text}");
            assert!(joined.contains("![literal](file.png)"), "{text}");
            assert_eq!(joined.contains("Partialfilesearch"), partial, "{text}");
            assert_eq!(joined.contains("resultLimitReached"), !partial, "{text}");
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom file search".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom file search"));
    }
}

// 내용 검색은 실제 match 제한과 행 잘림만 안내하고 오류·미지 메타데이터는 원문으로 남긴다.
// colon·Markdown 문자가 있는 결과를 파일 위치로 추측하지 않고 좁은 폭과 사용자 렌더러를 보존한다.
#[test]
fn content_search_renders_reported_limits_without_interpreting_result_lines() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (details, failed, limit, lines, fallback) in [
        (
            json!({"matchLimitReached": 2, "linesTruncated": true}),
            false,
            true,
            true,
            false,
        ),
        (json!({"linesTruncated": false}), false, false, false, false),
        (json!({"matchLimitReached": 1}), false, true, false, false),
        (
            json!({"matchLimitReached": u64::MAX}),
            false,
            true,
            false,
            false,
        ),
        (json!({"matchLimitReached": 0}), false, false, false, true),
        (json!({"matchLimitReached": -1}), false, false, false, true),
        (json!({"matchLimitReached": 1.5}), false, false, false, true),
        (json!({"linesTruncated": "yes"}), false, false, false, true),
        (
            json!({"matchLimitReached": 2, "future": 1}),
            false,
            false,
            false,
            true,
        ),
        (
            json!({"matchLimitReached": 2, "linesTruncated": true}),
            true,
            false,
            false,
            true,
        ),
    ] {
        let source = "src/a:b.rs:12: **literal** fn main()\nsrc/a:b.rs-13- ![literal](file.png)";
        let output = ToolOutput {
            tool: "grep".to_owned(),
            server: None,
            arguments: Some(
                json!({"pattern": "fn main", "path": "src", "glob": "*.rs", "ignoreCase": true}),
            ),
            result: Some(
                json!({"content": [{"type":"text","text": source}], "details": details, "isError": failed}),
            ),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text.split_whitespace().collect::<String>();
            assert!(joined.contains("Contentsearchpattern"), "{text}");
            assert!(joined.contains("ignoreCase"), "{text}");
            assert!(
                joined.contains("src/a:b.rs:12:**literal**fnmain()"),
                "{text}"
            );
            assert!(joined.contains("![literal](file.png)"), "{text}");
            assert_eq!(joined.contains("Searchresults"), !failed, "{text}");
            assert_eq!(joined.contains("Matchlimitreached:"), limit, "{text}");
            assert_eq!(joined.contains("Partialsearchlines"), lines, "{text}");
            assert_eq!(
                joined.contains("matchLimitReached") || joined.contains("linesTruncated"),
                fallback,
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom content search".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom content search"));
    }
}

// MCP 리소스 링크의 확인된 필드만 구분하고 미지 필드·원문·잘못된 값은 보존한다.
// 좁은 폭에서도 설명을 Markdown이나 이미지로 실행하지 않고 URI를 자동으로 열지 않는다.
#[test]
fn resource_links_preserve_metadata_literals_and_renderer_input() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (size, rich) in [
        (json!(0), true),
        (json!(u64::MAX), true),
        (json!(-1), false),
        (json!(1.5), false),
        (json!("20"), false),
    ] {
        let block = json!({"type":"resource_link","name":"report.txt","title":"Build report",
            "uri":"file:///unopened/report.txt","description":"**literal**\n![image](file.png)",
            "mimeType":"text/plain","size":size,"annotations":{"audience":["user"]},"future":"retained"});
        let output = ToolOutput {
            tool: "resources".to_owned(),
            server: Some("any-provider".to_owned()),
            arguments: None,
            result: Some(json!({"content":[block]})),
            content_items: None,
            error: None,
            plain_text: "Original resource link receipt".to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text.split_whitespace().collect::<String>();
            assert_eq!(joined.contains("Resourcelink"), rich, "{text}");
            assert!(joined.contains("file:///unopened/report.txt"), "{text}");
            assert!(joined.contains("**literal**"), "{text}");
            assert!(joined.contains("![image](file.png)"), "{text}");
            assert!(
                joined.contains("future")
                    && joined.contains("retained")
                    && joined.contains("audience"),
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert!(!frame.surface.has_hyperlinks());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom resource card".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom resource card"));
    }
}

// 리소스 카드의 JSON 256 KiB 경계와 첫 초과는 각각 카드와 전체 리터럴 fallback을 선택한다.
#[test]
fn resource_link_card_limit_preserves_the_first_excess_byte() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    for (bytes, rich) in [(256 * 1024, true), (256 * 1024 + 1, false)] {
        let mut block =
            json!({"type":"resource_link","name":"a","uri":"resource://a","description":""});
        let padding = bytes - block.to_string().len();
        block["description"] = "x".repeat(padding).into();
        assert_eq!(block.to_string().len(), bytes);
        let output = ToolOutput {
            tool: "resources".to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[block]})),
            content_items: None,
            error: None,
            plain_text: format!("Original {bytes}-byte resource receipt"),
        };
        let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(12));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Resource link"), rich, "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 내장 리소스는 URI·MIME·코드와 내외부 메타데이터를 보존하며 모호한 본문은 JSON으로 유지한다.
#[test]
fn embedded_resources_keep_outer_and_inner_metadata_across_resize() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (resource, rich) in [
        (
            json!({"uri":"resource://main.rs","mimeType":"text/x-rust","text":"fn main() {}\n![literal](file.png)","_meta":{"revision":7}}),
            true,
        ),
        (
            json!({"uri":"resource://data","mimeType":"application/octet-stream","blob":"AA==","_meta":{"revision":7}}),
            true,
        ),
        (
            json!({"uri":"resource://data","mimeType":12,"text":"literal","_meta":{"revision":7}}),
            false,
        ),
        (
            json!({"uri":"resource://data","text":"literal","blob":"AA==","_meta":{"revision":7}}),
            false,
        ),
        (
            json!({"uri":"","text":"literal","_meta":{"revision":7}}),
            false,
        ),
    ] {
        let output = ToolOutput {
            tool: "lookup".to_owned(),
            server: Some("generic".to_owned()),
            arguments: None,
            result: Some(
                json!({"content":[{"type":"resource","resource":resource,"annotations":{"audience":["user"]},"future":"retained"}]}),
            ),
            content_items: None,
            error: None,
            plain_text: "Original embedded resource".to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text.split_whitespace().collect::<String>();
            assert_eq!(joined.contains("Resourcemetadata"), rich, "{text}");
            assert!(
                joined.contains("revision")
                    && joined.contains("audience")
                    && joined.contains("future")
                    && joined.contains("retained"),
                "{text}"
            );
            if rich && resource.get("text").is_some() {
                assert!(joined.contains("fnmain(){}"), "{text}");
                assert!(joined.contains("![literal](file.png)"), "{text}");
                assert!(joined.contains("Type:text/x-rust"), "{text}");
            }
            if rich && resource.get("blob").is_some() {
                assert!(joined.contains("Binaryresource"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom embedded resource".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom embedded resource"));
    }
}

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

// 항목 수·경로 길이·입력 바이트 상한의 첫 초과 값은 일부만 목록화하지 않고 전체 원문으로 돌아간다.
#[test]
fn directory_listing_limits_fall_back_without_partial_parsing() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    let at_bytes = format!("{}\n", "p".repeat(1023)).repeat(256);
    let mut over_bytes = at_bytes.clone();
    over_bytes.insert(over_bytes.len() - 1, 'p');
    for (source, rich) in [
        ("a\n".repeat(1024), true),
        ("a\n".repeat(1025), false),
        (format!("{}\n", "p".repeat(1024)), true),
        (format!("{}\n", "p".repeat(1025)), false),
        (at_bytes, true),
        (over_bytes, false),
    ] {
        let output = ToolOutput {
            tool: "list_files".to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[{"type":"text","text":source}],"truncated":false})),
            content_items: None,
            error: None,
            plain_text: source,
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 60), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("file  "), rich, "{text}");
    }
}

// 셸 접기는 최신 출력과 명령·결과를 유지하며 스트리밍·폭 변경·설정·전체 펼치기를 반영한다.
#[test]
fn shell_tail_keeps_latest_output_and_restores_complete_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use super::key;
    use crate::{
        OutputPreferences, ToolRenderer,
        input::event::{KeyCode, KeyModifiers},
    };

    for tool in ["commandExecution", "run_command", "bash", "powershell"] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(3));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        let lines = (0..12)
            .map(|n| format!("output {n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = ToolOutput {
            tool: tool.to_owned(),
            server: None,
            arguments: Some(json!({"command":"echo test","cwd":"/workspace"})),
            result: None,
            content_items: None,
            error: None,
            plain_text: lines.clone(),
        };
        for latest in ["LATEST-A", "LATEST-B"] {
            let text = if tool == "run_command" {
                format!("status: 0\nstdout:\n{lines}\n{latest}\nstderr:\nwarning")
            } else {
                format!("{lines}\n{latest}")
            };
            output.result =
                Some(json!({"content":[{"type":"text","text":text}],"exitCode":0,"durationMs":7}));
            output.plain_text = text;
            session
                .parts_mut()
                .state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(1),
                    update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
                })
                .unwrap();
            let original = session.session_output().unwrap().unwrap();
            let pin = session.appearance_pin();
            for width in [80, 24, 80] {
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 100), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                let joined = text
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                assert!(text.contains(latest), "{text}");
                assert!(text.contains("echo test"), "{text}");
                assert!(!text.contains("output 00"), "{text}");
                assert!(joined.contains("earlieroutputrowshidden"), "{text}");
                assert!(text.contains("output 11"), "{text}");
                if tool == "commandExecution" {
                    assert!(text.contains("Duration: 7 ms"), "{text}");
                }
                assert_eq!(session.session_output().unwrap().unwrap(), original);
            }
        }
        let pin = session.appearance_pin();
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(
            text.contains("output 00") && text.contains("LATEST-B"),
            "{text}"
        );
        assert!(!text.contains("earlier output rows hidden"), "{text}");
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        session = session
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(u16::MAX));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("output 00"));
        session = session.with_output_preferences(
            OutputPreferences::default()
                .with_shell_tail_rows(0)
                .with_tool_head_rows(u16::MAX),
        );
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("output 00"));
        let expected = output.clone();
        session = session
            .with_output_preferences(
                OutputPreferences::default()
                    .with_shell_tail_rows(1)
                    .with_tool_head_rows(u16::MAX),
            )
            .with_tool_renderer(Some(ToolRenderer::new(move |input| {
                assert_eq!(input.output, Some(&expected));
                Some("CUSTOM FIRST\n\nCUSTOM LAST".to_owned())
            })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(
            text.contains("CUSTOM FIRST") && text.contains("CUSTOM LAST"),
            "{text}"
        );
    }
}

// 마지막 표시 행 수의 경계와 한글·탭·제어 문자·Markdown 원문이 접힌 화면에서도 안전하게 남는다.
#[test]
fn shell_tail_counts_visual_rows_and_keeps_literal_cells() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};
    for (source, hidden) in [
        ("one\ntwo\nthree", false),
        ("zero\none\ntwo\nthree", true),
        ("old\nold\nold\n\t한글\n![x](a.png)\n\u{1b}[31m", true),
    ] {
        let output = ToolOutput {
            tool: "commandExecution".to_owned(),
            server: None,
            arguments: Some(json!({"command":"test"})),
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(3));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 80), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("earlieroutputrowshidden"), hidden, "{text}");
            if source.contains("한글") {
                assert!(joined.contains("한글"), "{text}");
                assert!(text.contains("![x](a.png)"), "{text}");
                assert!(text.contains("^[[31m"), "{text}");
            } else {
                assert!(text.contains("one") && text.contains("three"), "{text}");
                assert!(!text.contains("zero"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
        }
        assert_eq!(session.session_output().unwrap().unwrap(), original);
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Original shell source retained".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 80), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Original shell source retained"));
    }
}

// 보고된 셸 잘림·전체 출력 경로를 표시하고 모호한 메타데이터와 원본 콜백을 보존한다.
#[test]
fn shell_output_details_show_reported_limits_and_literal_paths() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};
    let truncation = json!({"content":"tail", "truncated":true,"truncatedBy":"bytes","totalLines":100,"totalBytes":9000,"outputLines":1,"outputBytes":4,"lastLinePartial":true,"firstLineExceedsLimit":false,"maxLines":2000,"maxBytes":4});
    let mut lines = truncation.clone();
    lines["truncatedBy"] = json!("lines");
    lines["lastLinePartial"] = json!(false);
    lines["maxLines"] = json!(1);
    let mut complete = truncation.clone();
    complete["truncated"] = json!(false);
    complete["truncatedBy"] = json!(null);
    complete["lastLinePartial"] = json!(false);
    complete["totalLines"] = json!(1);
    complete["totalBytes"] = json!(4);
    let mut invalid = truncation.clone();
    invalid["outputBytes"] = json!(9001);
    let mut mismatch = truncation.clone();
    mismatch["content"] = json!("lost");
    for (details, rich, truncated) in [
        (
            json!({"truncation":truncation,"fullOutputPath":"/tmp/build output.log"}),
            true,
            true,
        ),
        (json!({"truncation":lines}), true, true),
        (json!({"truncation":complete}), true, false),
        (
            json!({"fullOutputPath":"![literal](file.png)"}),
            true,
            false,
        ),
        (json!({"truncation":invalid}), false, false),
        (json!({"truncation":mismatch}), false, false),
        (
            json!({"fullOutputPath":"/tmp/x","extension":"retained"}),
            false,
            false,
        ),
        (json!({"fullOutputPath":"/tmp/\u{1b}[31m"}), false, false),
        (json!({"fullOutputPath":"x".repeat(4096)}), true, false),
        (json!({"fullOutputPath":"x".repeat(4097)}), false, false),
    ] {
        let output = ToolOutput {
            tool: "bash".to_owned(),
            server: None,
            arguments: Some(json!({"command":"echo test"})),
            result: Some(json!({"content":[{"type":"text","text":"tail"}],"details":details})),
            content_items: None,
            error: None,
            plain_text: format!("tail\n{details}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(3));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 1000), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("Outputtruncated"), truncated, "{text}");
            assert_eq!(
                text.lines().any(|line| line.trim() == "details"),
                !rich,
                "{text}"
            );
            if rich && details.get("fullOutputPath").is_some() {
                assert!(joined.contains("Fulloutputfile"), "{text}");
            }
            if details.pointer("/truncation/lastLinePartial") == Some(&json!(true)) && rich {
                assert!(joined.contains("Lastlineispartial"), "{text}");
            }
            if rich && truncated {
                assert!(joined.contains("Outputlines:1of100"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Original details retained".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Original details retained"));
    }
}

// 메타데이터 표시 한도의 정확한 끝과 첫 초과 바이트에서 원문 fallback이 전환된다.
#[test]
fn shell_output_details_size_limit_preserves_literal_fallback() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;
    for extra in [0, 1] {
        let target = 256 * 1024 + extra;
        let mut content = "x".repeat(target - 300);
        let mut details = json!({"truncation":{"content":"","truncated":true,"truncatedBy":"bytes","totalLines":2,"totalBytes":999999,"outputLines":1,"outputBytes":0,"lastLinePartial":true,"firstLineExceedsLimit":false,"maxLines":2000,"maxBytes":999999}});
        for _ in 0..8 {
            details["truncation"]["content"] = json!(content);
            details["truncation"]["outputBytes"] = json!(content.len());
            let size = details.to_string().len();
            if size == target {
                break;
            }
            if size < target {
                content.push_str(&"x".repeat(target - size));
            } else {
                content.truncate(content.len() - (size - target));
            }
        }
        assert_eq!(details.to_string().len(), target);
        let output = ToolOutput {
            tool: "bash".to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[{"type":"text","text":content}],"details":details})),
            content_items: None,
            error: None,
            plain_text: "Original large metadata retained".to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(1));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 4000), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Output truncated"), extra == 0);
        assert_eq!(
            text.lines().any(|line| line.trim() == "details"),
            extra == 1
        );
    }
}

// 내장 명령의 진행 결과는 종료 코드를 꾸미지 않고 스트림별로 표시되며 최종 출력으로 교체된다.
#[test]
fn native_command_progress_renders_streams_until_authoritative_completion() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(5));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    for completed in [false, true] {
        let output = ToolOutput {
            tool: "run_command".to_owned(),
            server: None,
            arguments: Some(json!({"command":"echo test"})),
            result: Some(if completed {
                json!({"content":[{"type":"text","text":"status: 0\nstdout:\nfinal output\nstderr:\n"}]})
            } else {
                json!({"content":[{"type":"text","text":json!({"stdout":"first output\n![literal](file.png)","stderr":"warning"}).to_string()}],"progress":true})
            }),
            content_items: None,
            error: None,
            plain_text: if completed {
                "final output".to_owned()
            } else {
                "first output".to_owned()
            },
        };
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(text.contains("stdout") && text.contains("stderr"), "{text}");
            assert_eq!(joined.contains("Exitstatus:0"), completed, "{text}");
            assert_eq!(text.contains("final output"), completed, "{text}");
            assert_eq!(text.contains("first output"), !completed, "{text}");
            assert_eq!(text.contains("![literal]"), !completed, "{text}");
            assert!(!text.contains("\"progress\""), "{text}");
            assert!(frame.surface.rasters.is_empty());
        }
    }
}

// 7만 행과 좁은 화면에서 6만 행 넘게 개행되는 한 줄도 최신 출력·원문·사용자 콜백을 보존한다.
#[test]
fn folded_shell_output_survives_large_row_counts_and_long_lines() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};
    for source in [
        format!("EARLY\n{}LATEST", "row\n".repeat(70_000)),
        format!("EARLY{}LATEST", "x".repeat(1_500_000)),
    ] {
        let output = ToolOutput {
            tool: "commandExecution".to_owned(),
            server: None,
            arguments: Some(json!({"command":"emit test"})),
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: source.clone(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(5));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        for columns in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(columns, 80), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(joined.contains("LATEST"), "{text}");
            if source.contains('\n') {
                assert!(joined.contains("69997earlieroutputrows"), "{text}");
            }
            assert!(!text.contains("EARLY"), "{text}");
            assert!(joined.contains("earlieroutputrowshidden"), "{text}");
            assert!(text.contains("emit test"), "{text}");
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Full large source available".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Full large source available"));
    }
}

// 검색·페이지 열기·찾기는 명확한 제목과 원문 패널로 표시하고 URL/패턴을 Markdown으로 실행하지
// 않는다. 테마·폭·콜백 변경 뒤에도 구조화 인수와 전체 원문을 보존한다.
#[test]
fn web_actions_render_literal_details_and_allow_typed_customization() {
    use std::num::NonZeroU16;

    use serde_json::json;
    use yo_core::{ActivityOutcome, ToolOutput};

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (heading, action, detail) in [
        (
            "Web search",
            json!({"type":"search","queries":["Rust 한글","configurable UI"]}),
            "Query: Rust 한글\nQuery: configurable UI",
        ),
        (
            "Open web page",
            json!({"type":"openPage","url":"https://example.com/docs"}),
            "URL: https://example.com/docs",
        ),
        (
            "Find in web page",
            json!({"type":"findInPage","pattern":"![literal](data:image/png;base64,AAAA)\n```rust\nunsafe text"}),
            "Find: ![literal](data:image/png;base64,AAAA)\n```rust\nunsafe text",
        ),
        ("Web search", json!(null), "Details not reported"),
    ] {
        let output = ToolOutput {
            tool: "webSearch".into(),
            server: None,
            arguments: Some(json!({"query":"fallback", "action":action})),
            result: None,
            content_items: None,
            error: None,
            plain_text: format!("{heading}\n{detail}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_max_body_width(NonZeroU16::new(32)),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(11, 22, 33)),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 50), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(joined.contains(&heading.replace(' ', "")), "{text}");
            assert!(
                joined.contains(
                    &detail
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>()
                ),
                "{text}"
            );
            assert!(
                !text.contains("Arguments") && !text.contains("yo.tool-output"),
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert!(
                (0..frame.surface.size().height).any(|y| (0..frame.surface.size().width).any(
                    |x| {
                        frame
                            .surface
                            .cell(Point::new(x, y))
                            .unwrap()
                            .style()
                            .background
                            == Color::Rgb {
                                red: 11,
                                green: 22,
                                blue: 33,
                            }
                    }
                ))
            );
        }
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let pin = session.appearance_pin();
        let completed = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 50), &pin)
            .unwrap();
        let text = visible_rows(&completed.surface);
        assert!(text.contains("Tool completed"), "{text}");
        assert!(!text.contains("Tool call prepared"), "{text}");
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            assert_eq!(input.source, expected.plain_text);
            assert!(input.columns.get() <= 32);
            Some("Custom search presentation".into())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 20), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom search presentation"));
    }
}

// Markdown 링크 목적지는 문단·강조·인라인 코드·표의 개행과 좁은 폭에서도 셀에 보존된다.
// 클릭 설정을 끄면 글자·원문·테마를 바꾸지 않고 링크 정보만 제거한다.
#[test]
fn markdown_hyperlinks_survive_reflow_and_can_be_disabled() {
    use crate::OutputPreferences;
    let source = "[**한글** `code`](https://example.com/docs) after\n\n| Name | Link |\n| --- | --- |\n| one | [table](https://example.com/table) |\n\n```text\n[code](https://example.com/not-a-link)\n```\n\n[unsafe](javascript:alert(1)) [local](file:///tmp/a)";
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source.into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for width in [80, 24, 80] {
        let pin = session.appearance_pin();
        let linked = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 80), &pin)
            .unwrap();
        let mut linked_text = String::new();
        let mut destinations = Vec::new();
        for y in 0..linked.surface.size().height {
            for x in 0..linked.surface.size().width {
                let cell = linked.surface.cell(Point::new(x, y)).unwrap();
                if let Some(link) = cell.hyperlink() {
                    destinations.push(link.destination().to_owned());
                    if let CellContent::Grapheme { text, .. } = cell.content() {
                        linked_text.push_str(text);
                    }
                }
            }
        }
        assert!(linked_text.contains("한글"), "{linked_text}");
        assert!(linked_text.contains("code"), "{linked_text}");
        assert!(linked_text.contains("table"), "{linked_text}");
        assert!(!linked_text.contains("after"), "{linked_text}");
        assert!(
            destinations
                .iter()
                .any(|url| url == "https://example.com/docs")
        );
        assert!(
            destinations
                .iter()
                .any(|url| url == "https://example.com/table")
        );
        assert!(
            destinations
                .iter()
                .all(|url| url == "https://example.com/docs" || url == "https://example.com/table")
        );
        session =
            session.with_output_preferences(OutputPreferences::default().with_hyperlinks(false));
        let pin = session.appearance_pin();
        let unlinked = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 80), &pin)
            .unwrap();
        assert!(!unlinked.surface.has_hyperlinks());
        for y in 0..linked.surface.size().height {
            for x in 0..linked.surface.size().width {
                let before = linked.surface.cell(Point::new(x, y)).unwrap();
                let after = unlinked.surface.cell(Point::new(x, y)).unwrap();
                assert_eq!(before.content(), after.content());
                assert_eq!(before.style(), after.style());
            }
        }
        assert_eq!(session.session_output().unwrap().unwrap(), original);
        session =
            session.with_output_preferences(OutputPreferences::default().with_hyperlinks(true));
    }
}

// 시작·재개 안내는 실제 frame에서 좁은 폭에도 보이며 재그리기마다 중복되거나 Turn을 시작하지
// 않는다.
#[test]
fn host_startup_notice_survives_width_changes_without_starting_work() {
    for resumed in [false, true] {
        let mut session = TuiSession::with_session_info(
            GlyphProfile::Rich,
            TuiSessionInfo::new("host:demo · reported-model", "~/yo").with_startup_notice(resumed),
            ColorCapability::TrueColor,
            MotionPreference::Reduced,
        );
        let title = if resumed {
            "Session resumed"
        } else {
            "New session"
        };
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 15), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            assert!(text.contains(title), "{text}");
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("Backend:host:demo·reported-model"),
                "{text}"
            );
            assert!(text.contains("Workspace: ~/yo"), "{text}");
            assert!(!session.parts_mut().state.turn_active());
            assert!(frame.motion_demand.is_none());
        }
        let output = session.session_output().unwrap().unwrap();
        assert_eq!(output.matches(title).count(), 1);
    }
}

// 질문·승인 요청과 응답은 활동 테마와 단어 줄바꿈을 사용하며 원문과 결과를 보존한다.
// 사용자 Markdown/이미지/링크 문자열은 실행 가능한 표시가 아닌 그대로의 답으로 남는다.
#[test]
fn interaction_text_uses_activity_theme_and_literal_reflow_without_tool_callbacks() {
    use std::num::NonZeroU64;

    use yo_core::{ActivityOutcome, Failure, RequestId};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};
    let source = "Question 1 of 2\nWhich area?\n\nAnswer: **Runtime**\nNote: [literal](https://example.com)\n![x](data:image/png;base64,abc)\n\nRecorded; waiting for the remaining questions.";
    let request_id = RequestId::new(NonZeroU64::new(7).unwrap());
    for (kind, heading, source, expected) in [
        (
            ActivityKind::UserInputResponse { request_id },
            "Answer recorded",
            source,
            "Answer:**Runtime**",
        ),
        (
            ActivityKind::ApprovalResponse { request_id },
            "Approval response sent",
            "Decision: approved",
            "Decision:approved",
        ),
        (
            ActivityKind::ApprovalResponse { request_id },
            "Approval response sent",
            "Decision: declined",
            "Decision:declined",
        ),
        (
            ActivityKind::ApprovalRequest { request_id },
            "Approval required",
            "Working directory: /workspace/yo\n    Preserve indentation\n**Literal** request",
            "Workingdirectory:/workspace/yo",
        ),
        (
            ActivityKind::UserInputRequest { request_id },
            "Answer requested",
            source,
            "Answer:**Runtime**",
        ),
    ] {
        for outcome in [
            ActivityOutcome::Completed,
            ActivityOutcome::Failed(Failure::new("receipt interrupted")),
        ] {
            let mut session =
                TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
                    .with_theme_overrides(
                        ThemeOverrides::default()
                            .with_color(ThemeRole::Success, ThemeColor::Rgb(80, 180, 90)),
                    )
                    .with_tool_renderer(Some(ToolRenderer::new(|_| {
                        panic!("responses never enter tool customization")
                    })));
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
                    update: ActivityUpdate::TextSnapshot(source.into()),
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: outcome.clone(),
                })
                .unwrap();
            let pin = session.appearance_pin();
            for width in [80, 24, 80] {
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 50), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                if matches!(kind, ActivityKind::ApprovalRequest { .. }) {
                    assert!(
                        text.lines().any(|line| line.contains("/workspace/yo")),
                        "{text}"
                    );
                }
                let compact: String = text.split_whitespace().collect();
                assert!(
                    compact.contains(&heading.split_whitespace().collect::<String>()),
                    "{text}"
                );
                assert!(compact.contains(expected), "{text}");
                let y = (0..50)
                    .find(|y| {
                        (0..width)
                            .filter_map(|x| {
                                match frame.surface.cell(Point::new(x, *y)).unwrap().content() {
                                    CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                                    _ => None,
                                }
                            })
                            .collect::<String>()
                            .contains(heading.split_whitespace().next().unwrap())
                    })
                    .unwrap();
                let style = frame.surface.cell(Point::new(2, y)).unwrap().style();
                assert!(style.attributes.contains(Attributes::BOLD));
                if outcome == ActivityOutcome::Completed {
                    assert_eq!(
                        style.foreground,
                        Color::Rgb {
                            red: 80,
                            green: 180,
                            blue: 90
                        }
                    );
                }
                for y in 0..50 {
                    for x in 0..width {
                        assert!(
                            frame
                                .surface
                                .cell(Point::new(x, y))
                                .unwrap()
                                .hyperlink()
                                .is_none()
                        );
                    }
                }
            }
            let plain = session.session_output().unwrap().unwrap();
            let unindented = plain
                .lines()
                .map(|line| line.strip_prefix("  ").unwrap_or(line))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(unindented.contains(source), "{plain}");
            if matches!(outcome, ActivityOutcome::Failed(_)) {
                assert!(plain.contains("Failed: receipt interrupted"));
            }
        }
    }
}

// 스트리밍으로 정의가 도착하면 각주를 연결하되 도구 로그와 원문 내보내기는 변환하지 않는다.
#[test]
fn footnote_frames_preserve_streaming_source_and_tool_literals() {
    for kind in [ActivityKind::AgentMessage, ActivityKind::ToolResult] {
        let assistant = kind == ActivityKind::AgentMessage;
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("Claim[^note].".into()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let initial = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 20), &pin)
            .unwrap();
        assert!(visible_rows(&initial.surface).contains("Claim[^note]"));
        let source = "Claim[^note].\n\n[^note]: A **bold** source.";
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(source.into()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 24), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            assert!(
                text.contains(if assistant {
                    "Claim[note]"
                } else {
                    "Claim[^note]"
                }),
                "{text}"
            );
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        assert!(original.contains("Claim[^note]"));
        assert!(original.contains("[^note]: A **bold** source."));
    }
}

// 커스텀 렌더러는 실제 펼침 상태를 받고 왕복 토글·폭 변경 때 재배치되며 원문은 바꾸지 않는다.
#[test]
fn tool_renderer_receives_expansion_state_across_cached_frames() {
    use std::time::Duration;

    use super::key;
    use crate::{
        ToolRenderer,
        input::event::{KeyCode, KeyModifiers},
    };

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_tool_renderer(Some(ToolRenderer::new(|input| {
            assert_eq!(input.source, "retained original payload");
            Some(
                if input.expanded {
                    "Expanded detail"
                } else {
                    "Compact summary"
                }
                .into(),
            )
        })));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("retained original payload".into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for (toggle, width, expanded) in [
        (false, 80, false),
        (false, 80, false),
        (true, 80, true),
        (false, 24, true),
        (true, 24, false),
    ] {
        if toggle {
            session
                .parts_mut()
                .state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 24), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Expanded detail"), expanded, "{text}");
        assert_eq!(text.contains("Compact summary"), !expanded, "{text}");
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    assert!(original.contains("retained original payload"));
}

// 종료 결과는 payload 단어에서 추측하지 않고 실제 완료 이벤트 뒤 콜백·캐시에 반영한다.
#[test]
fn tool_renderer_receives_observed_terminal_outcomes() {
    use yo_core::{ActivityOutcome, Failure};

    use crate::{ToolRenderer, TranscriptActivityOutcome};

    for (outcome, expected) in [
        (ActivityOutcome::Completed, "Completion detail"),
        (ActivityOutcome::Interrupted, "Interruption detail"),
        (
            ActivityOutcome::Failed(Failure::new("Original failure")),
            "Failure detail",
        ),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(|input| {
                assert_eq!(input.source, "payload says completed and failed");
                Some(
                    match input.outcome {
                        None => "Waiting for outcome",
                        Some(TranscriptActivityOutcome::Completed) => "Completion detail",
                        Some(TranscriptActivityOutcome::Interrupted) => "Interruption detail",
                        Some(TranscriptActivityOutcome::Failed) => "Failure detail",
                    }
                    .into(),
                )
            })));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("payload says completed and failed".into()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 24), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Waiting for outcome"));
        session.parts_mut().state.commit_frame(&frame);
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome,
            })
            .unwrap();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 24), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            assert!(text.contains(expected), "{text}");
            assert!(!text.contains("Waiting for outcome"), "{text}");
            if expected == "Failure detail" {
                assert!(
                    text.split_whitespace()
                        .collect::<String>()
                        .contains("Originalfailure"),
                    "{text}"
                );
            }
            session.parts_mut().state.commit_frame(&frame);
        }
        let source = session.session_output().unwrap().unwrap();
        assert!(source.contains("payload says completed and failed"));
        assert!(!source.contains(expected));
    }
}

// 공통 diff 콘텐츠는 프로바이더 이름 없이 추가·삭제 테마를 쓰고 좁은 화면에서도 원문을 보존한다.
#[test]
fn generic_diff_content_uses_theme_and_preserves_plain_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    let patch = "--- old\n+++ new\n@@ -1 +1 @@\n-old value\n+new value\n";
    let output = ToolOutput {
        tool: "workspace_patch".into(),
        server: None,
        arguments: None,
        result: None,
        content_items: Some(
            json!([{"type":"diff","title":"File change · file.rs","text":patch,"source":{"oldText":"old value\n","newText":"new value\n"}}]),
        ),
        error: None,
        plain_text: patch.into(),
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
        .with_theme_overrides(
            ThemeOverrides::default()
                .with_color(ThemeRole::DiffAdded, ThemeColor::Rgb(20, 214, 120))
                .with_color(ThemeRole::DiffRemoved, ThemeColor::Rgb(215, 21, 120)),
        );
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let pin = session.appearance_pin();
    for width in [80, 24, 80] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        for (needle, color) in [
            (
                "-old value",
                Color::Rgb {
                    red: 215,
                    green: 21,
                    blue: 120,
                },
            ),
            (
                "+new value",
                Color::Rgb {
                    red: 20,
                    green: 214,
                    blue: 120,
                },
            ),
        ] {
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, row)| row.contains(needle))
                .unwrap();
            let x = row.find(needle).unwrap();
            assert_eq!(
                frame
                    .surface
                    .cell(Point::new(x as u16, y as u16))
                    .unwrap()
                    .style()
                    .foreground,
                color
            );
        }
        assert!(!text.contains("oldText"), "{text}");
        session.parts_mut().state.commit_frame(&frame);
    }
    assert!(
        session
            .session_output()
            .unwrap()
            .unwrap()
            .contains("-old value")
    );
}

// 공개 추론 색상은 숨김 안내에도 적용하지만 압축 요약·원문과 다른 테마 역할을 바꾸지 않는다.
#[test]
fn reasoning_color_is_independent_and_survives_resize_and_visibility() {
    use yo_core::{ActivitySummary, SummaryKind};

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    for visible in [true, false] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_reasoning(visible)
                    .with_tool_head_rows(u16::MAX),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Muted, ThemeColor::Rgb(60, 70, 80))
                    .with_color(ThemeRole::ReasoningText, ThemeColor::Rgb(152, 118, 84)),
            );
        for (id, kind, summary) in [
            (1, SummaryKind::Reasoning, "Reasoning words"),
            (2, SummaryKind::Compaction, "Compaction words"),
        ] {
            let state = session.parts_mut().state;
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: activity(id),
                    kind: ActivityKind::ModelWork,
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(id),
                    update: ActivityUpdate::TextSnapshot(
                        ActivitySummary {
                            kind,
                            summary: summary.into(),
                            tokens_before: None,
                        }
                        .to_snapshot()
                        .unwrap(),
                    ),
                })
                .unwrap();
        }
        let source = session.session_output().unwrap().unwrap();
        assert!(source.contains("Reasoning words"));
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            for (needle, color) in [
                (
                    if visible {
                        "Reasoning words"
                    } else {
                        "Summary hidden"
                    },
                    Color::Rgb {
                        red: 152,
                        green: 118,
                        blue: 84,
                    },
                ),
                (
                    "Compaction words",
                    Color::Rgb {
                        red: 60,
                        green: 70,
                        blue: 80,
                    },
                ),
            ] {
                let (y, row) = text
                    .lines()
                    .enumerate()
                    .find(|(_, row)| row.contains(needle))
                    .unwrap();
                let x = row.find(needle).unwrap();
                assert_eq!(
                    frame
                        .surface
                        .cell(Point::new(x as u16, y as u16))
                        .unwrap()
                        .style()
                        .foreground,
                    color
                );
            }
            session.parts_mut().state.commit_frame(&frame);
        }
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 문서 콜백은 문서에만 적용되고 펼침·폭·실패 상태를 받으며 교체·실패 대체·원문 보존을 지원한다.
#[test]
fn document_renderer_preserves_ownership_source_and_fallback() {
    use yo_core::{ActivityDocument, ActivityOutcome, ActivitySummary, Failure, SummaryKind};

    use super::key;
    use crate::{
        DocumentRenderer, OutputPreferences, TranscriptActivityOutcome,
        input::event::{KeyCode, KeyModifiers},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
        .with_document_renderer(Some(DocumentRenderer::new(|input| {
            assert_eq!(input.document.title, "Host document");
            assert_eq!(input.document.markdown, "Original document body");
            assert_eq!(input.outcome, Some(TranscriptActivityOutcome::Failed));
            assert!(input.columns.get() <= 80);
            Some(
                if input.expanded {
                    "Expanded detail"
                } else {
                    "Compact summary"
                }
                .into(),
            )
        })));
    for (id, source) in [
        (
            1,
            ActivityDocument {
                title: "Host document".into(),
                markdown: "Original document body".into(),
            }
            .to_snapshot()
            .unwrap(),
        ),
        (
            2,
            ActivitySummary {
                kind: SummaryKind::Reasoning,
                summary: "Keep public summary".into(),
                tokens_before: None,
            }
            .to_snapshot()
            .unwrap(),
        ),
    ] {
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(source),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(id),
                outcome: ActivityOutcome::Failed(Failure::new("Retained failure")),
            })
            .unwrap();
    }
    let original = session.session_output().unwrap().unwrap();
    for (toggle, width, expanded) in [
        (false, 80, false),
        (false, 80, false),
        (true, 80, true),
        (false, 24, true),
        (true, 24, false),
    ] {
        if toggle {
            session
                .parts_mut()
                .state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(!text.contains("yo.activity-document"));
        assert!(text.contains("Host document"));
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains("Retainedfailure"),
            "{text}"
        );
        assert!(text.contains("Keep public summary"));
        assert_eq!(text.contains("Expanded detail"), expanded);
        assert_eq!(text.contains("Compact summary"), !expanded);
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    for (renderer, expected) in [
        (
            Some(DocumentRenderer::new(|_| Some("Replacement body".into()))),
            "Replacement body",
        ),
        (
            Some(DocumentRenderer::new(|_| Some("x\n\n".repeat(32769)))),
            "Original document body",
        ),
        (
            Some(DocumentRenderer::new(|_| Some("x\n\n".repeat(32768)))),
            "Original document body",
        ),
        (
            Some(DocumentRenderer::new(|_| None)),
            "Original document body",
        ),
        (None, "Original document body"),
    ] {
        session = session.with_document_renderer(renderer);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(!text.contains("yo.activity-document"));
        assert!(text.contains(expected), "{text}");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains("Retainedfailure"),
            "{text}"
        );
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 완료된 사용량만 짧게 접고 여러 프로바이더의 상세·개별 관측·원문은 펼침과 폭 변경 뒤에도 보존한다.
#[test]
fn completed_usage_compacts_without_losing_observations_or_source() {
    use serde_json::json;
    use yo_core::ActivityOutcome;

    use super::key;
    use crate::{
        OutputPreferences,
        input::event::{KeyCode, KeyModifiers},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default());
    for (id, schema, profile, input) in [
        (
            1,
            "codex.app-server-token-usage-receipt/v1",
            "codex.app-server.thread-token-usage-updated/v1",
            120,
        ),
        (
            2,
            "grok.acp-prompt-usage-receipt/v1",
            "grok.acp.prompt-response.usage/v1",
            240,
        ),
    ] {
        let receipt=json!({"schema":schema,"source_profile":profile,"turn_id":"turn-a","prompt_request_id":42,
            "usage":{"input_tokens":input,"output_tokens":30,"total_tokens":input+30,"reasoning_tokens":12,"cache_read_input_tokens":80,"cache_write_input_tokens":0}}).to_string();
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(receipt),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(id),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }
    let original = session.session_output().unwrap().unwrap();
    assert_eq!(original.matches("Cache read: 80").count(), 2);
    for (toggle, width, expanded) in [
        (false, 80, false),
        (false, 24, false),
        (true, 24, true),
        (false, 80, true),
        (true, 80, false),
    ] {
        if toggle {
            session
                .parts_mut()
                .state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        let joined = text.split_whitespace().collect::<String>();
        assert!(joined.contains("Input:120"));
        assert!(joined.contains("Input:240"));
        assert_eq!(joined.contains("Cacheread:80"), expanded);
        assert_eq!(
            text.matches("Usage ·").count(),
            if expanded { 0 } else { 2 }
        );
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 호스트 상태는 키 순서대로 별도 행에 표시하고 좁은 폭에서는 grapheme을 자르지 않고 생략한다.
// 갱신·삭제·테마 변경은 대화 원문을 바꾸지 않고 낮은 화면의 입력과 대화 공간을 보존한다.
#[test]
fn host_status_line_updates_without_changing_conversation_source() {
    use crate::{
        ThemeColor, ThemeOverrides, ThemeRole, TuiStatusLine,
        runner::{AgentPoll, unix::apply_agent_poll},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("Preserved conversation".into()),
        })
        .unwrap();
    let source = session.session_output().unwrap();
    assert!(source.as_ref().unwrap().contains("Preserved conversation"));
    let status = TuiStatusLine::new([
        ("z-worker", "Worker: ready"),
        ("a-checks", "Checks: 한글 passed"),
    ])
    .unwrap();
    assert!(
        apply_agent_poll(
            session.parts_mut().state,
            AgentPoll::StatusLine(status.clone())
        )
        .unwrap()
    );
    assert!(!apply_agent_poll(session.parts_mut().state, AgentPoll::StatusLine(status)).unwrap());
    for width in [80, 24, 3, 80] {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 30), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        if width == 80 {
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("Checks:한글passed·Worker:ready")
            );
        } else if width == 24 {
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("Checks:한글passed")
            );
            assert!(text.contains("..."));
        } else {
            assert!(text.contains('.'));
        }
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap(), source);
    }
    session = session.with_theme_overrides(
        ThemeOverrides::default().with_color(ThemeRole::Muted, ThemeColor::Rgb(12, 34, 56)),
    );
    assert!(session.set_status_line(TuiStatusLine::new([("checks", "Checks: updated")]).unwrap()));
    let pin = session.appearance_pin();
    let updated = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(visible_rows(&updated.surface).contains("Checks: updated"));
    assert_eq!(
        updated
            .surface
            .cell(Point::new(0, 28))
            .unwrap()
            .style()
            .foreground,
        Color::Rgb {
            red: 12,
            green: 34,
            blue: 56
        }
    );
    assert!(!visible_rows(&updated.surface).contains("Worker: ready"));
    session.parts_mut().state.commit_frame(&updated);
    assert!(session.set_status_line(TuiStatusLine::default()));
    let empty = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(!visible_rows(&empty.surface).contains("Checks:"));
    assert_eq!(session.session_output().unwrap(), source);
}

// 호스트가 확인한 명시적 Markdown 링크만 파일 목적지로 바꾸며 코드·이미지·원문은 그대로 둔다.
// 폭 변경, resolver 교체·제거, 링크 비활성화 후에도 기본 웹 링크와 원문 보존을 검증한다.
#[test]
fn host_link_resolver_preserves_default_web_links_and_source() {
    use std::{
        path::Path,
        sync::{Arc, Mutex},
    };

    use crate::{LinkResolver, OutputPreferences, surface::Hyperlink};
    let file = Hyperlink::from_file_path(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .canonicalize()
            .unwrap(),
    )
    .unwrap();
    let replacement = Hyperlink::from_file_path(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/lib.rs")
            .canonicalize()
            .unwrap(),
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let target = file.clone();
    let resolver = LinkResolver::new(move |destination| {
        observed.lock().unwrap().push(destination.to_owned());
        (destination == "Cargo.toml").then(|| target.clone())
    });
    let source = "[**한글** `manifest`](Cargo.toml) [web](https://example.com/docs)\n\n| Name | File |\n| --- | --- |\n| row | [table](Cargo.toml) |\n\n```text\n[code](code-only)\n```\n\n![image](image-only) [unmapped](missing.rs) [raw](file:///tmp/untrusted)";
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_link_resolver(Some(resolver));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source.into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(calls.lock().unwrap().is_empty());
    for width in [80, 24, 80] {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 90), &pin)
            .unwrap();
        let mut linked_text = String::new();
        let mut destinations = Vec::new();
        for y in 0..frame.surface.size().height {
            for x in 0..frame.surface.size().width {
                let cell = frame.surface.cell(Point::new(x, y)).unwrap();
                if let Some(link) = cell.hyperlink() {
                    destinations.push(link.destination().to_owned());
                    if link == &file
                        && let CellContent::Grapheme { text, .. } = cell.content()
                    {
                        linked_text.push_str(text);
                    }
                }
            }
        }
        assert!(linked_text.contains("한글"));
        assert!(linked_text.contains("manifest"));
        assert!(linked_text.contains("table"));
        assert!(
            destinations
                .iter()
                .any(|value| value == "https://example.com/docs")
        );
        assert!(
            destinations
                .iter()
                .all(|value| value == file.destination() || value == "https://example.com/docs")
        );
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|value| value != "code-only" && value != "image-only")
    );
    let next = replacement.clone();
    session = session.with_link_resolver(Some(LinkResolver::new(move |destination| {
        (destination == "Cargo.toml").then(|| next.clone())
    })));
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 90), &pin)
        .unwrap();
    let links = |surface: &Surface| {
        (0..surface.size().height)
            .flat_map(|y| {
                (0..surface.size().width).filter_map(move |x| {
                    surface.cell(Point::new(x, y)).unwrap().hyperlink().cloned()
                })
            })
            .collect::<Vec<_>>()
    };
    assert!(links(&frame.surface).contains(&replacement));
    assert!(!links(&frame.surface).contains(&file));
    session.parts_mut().state.commit_frame(&frame);
    session = session.with_link_resolver(Some(LinkResolver::new(|_| {
        panic!("disabled links must not resolve")
    })));
    session = session.with_output_preferences(OutputPreferences::default().with_hyperlinks(false));
    let pin = session.appearance_pin();
    let disabled = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 90), &pin)
        .unwrap();
    assert!(!disabled.surface.has_hyperlinks());
    assert_eq!(
        visible_rows(&disabled.surface),
        visible_rows(&frame.surface)
    );
    assert_eq!(session.session_output().unwrap().unwrap(), original);
    session = session
        .with_link_resolver(None)
        .with_output_preferences(OutputPreferences::default().with_hyperlinks(true));
    let pin = session.appearance_pin();
    let default = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 90), &pin)
        .unwrap();
    assert!(
        links(&default.surface)
            .iter()
            .all(|link| link.destination() == "https://example.com/docs")
    );
}

// Alt+O는 반영된 화면의 항목만 토글하고 개별 상태·렌더러 인수·원문을 유지하며 Ctrl+O는 전체를
// 재설정한다.
#[test]
fn individual_activity_expansion_preserves_other_items_and_custom_rendering() {
    use std::time::Duration;

    use yo_core::ActivityOutcome;

    use super::key;
    use crate::{
        ToolRenderer,
        input::event::{KeyCode, KeyModifiers},
    };

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_tool_renderer(Some(ToolRenderer::new(|input| {
            Some(format!(
                "{} {}",
                input.source,
                if input.expanded {
                    "expanded"
                } else {
                    "compact"
                }
            ))
        })));
    for (index, text) in [(1, "FIRST"), (2, "SECOND")] {
        let state = &mut session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(index),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(index),
                update: ActivityUpdate::TextSnapshot(text.into()),
            })
            .unwrap();
    }
    for index in [1, 2] {
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(index),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }
    let original = session.session_output().unwrap().unwrap();
    let render = |session: &mut TuiSession, width| {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 32), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        session.parts_mut().state.commit_frame(&frame);
        text
    };
    render(&mut session, 80);
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let pin = session.appearance_pin();
    let protected = session
        .parts_mut()
        .state
        .prepare_frame_for_geometry(Size::new(80, 32), &pin, Duration::ZERO, 1)
        .unwrap();
    assert!(protected.publication.is_none());
    for width in [80, 24, 80] {
        let text = render(&mut session, width);
        assert!(text.contains("FIRST compact"), "{text}");
        assert!(text.contains("SECOND expanded"), "{text}");
    }
    session
        .parts_mut()
        .state
        .handle(key(KeyCode::Home, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    // 화면 반영 전 이동과 토글이 겹치면 이전 항목을 잘못 바꾸지 않는다.
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 80);
    assert!(text.contains("FIRST compact"));
    assert!(text.contains("SECOND expanded"));
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 24);
    assert!(text.contains("FIRST expanded"));
    assert!(text.contains("SECOND expanded"));
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 80);
    assert!(text.contains("FIRST compact"));
    assert!(text.contains("SECOND expanded"));
    for expanded in [true, false] {
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let text = render(&mut session, 80);
        for name in ["FIRST", "SECOND"] {
            assert!(
                text.contains(&format!(
                    "{name} {}",
                    if expanded { "expanded" } else { "compact" }
                )),
                "{text}"
            );
        }
    }
    session
        .parts_mut()
        .state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let pin = session.appearance_pin();
    let resumed = session
        .parts_mut()
        .state
        .prepare_frame_for_geometry(Size::new(80, 32), &pin, Duration::ZERO, 1)
        .unwrap();
    assert!(resumed.publication.is_some());
    assert_eq!(session.session_output().unwrap().unwrap(), original);
}

// 기본 로그의 개별 펼침은 스트리밍 revision과 실패 footer를 보존하고 이후 항목까지 펼치지 않는다.
#[test]
fn individual_default_log_expansion_survives_streaming_and_failure() {
    use std::time::Duration;

    use yo_core::{ActivityOutcome, Failure};

    use super::key;
    use crate::input::event::{KeyCode, KeyModifiers};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let payload = |prefix: &str, count| {
        (1..=count)
            .map(|n| format!("{prefix} {n:02}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let render = |session: &mut TuiSession, width| {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 90), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        session.parts_mut().state.commit_frame(&frame);
        text
    };
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(payload("FIRST", 20)),
        })
        .unwrap();
    let compact = render(&mut session, 80);
    assert!(!compact.contains("FIRST 05"));
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert!(render(&mut session, 80).contains("FIRST 05"));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(payload("FIRST", 24)),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("Failure retained")),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(2),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(2),
            update: ActivityUpdate::TextSnapshot(payload("SECOND", 20)),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for width in [80, 24, 80] {
        let text = render(&mut session, width);
        assert!(text.contains("FIRST 05"), "{text}");
        assert!(text.contains("FIRST 24"), "{text}");
        assert!(
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .contains("Failed:Failureretained"),
            "{text}"
        );
        assert!(!text.contains("SECOND 05"), "{text}");
        assert!(text.contains("SECOND 20"), "{text}");
    }
    // 최신 항목 토글도 기존 첫 항목의 선택을 잃지 않는다.
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 24);
    assert!(text.contains("FIRST 05"));
    assert!(text.contains("SECOND 05"));
    assert_eq!(session.session_output().unwrap().unwrap(), original);
    assert!(original.contains("FIRST 05"));
    assert!(original.contains("SECOND 05"));
}

// 호스트 문서는 실제 poll 경계에서 Turn 없이 나타나고 커스텀 문서 렌더러·원문·개별 펼침을 유지한다.
#[test]
fn host_document_poll_renders_without_a_turn_and_keeps_original() {
    use std::time::Duration;

    use yo_core::ActivityDocument;

    use super::key;
    use crate::{
        DocumentRenderer, TuiDocument,
        input::event::{KeyCode, KeyModifiers},
        runner::{AgentPoll, unix::apply_agent_poll},
    };

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_document_renderer(Some(DocumentRenderer::new(|input| {
            assert_eq!(input.document.title, "Host guide");
            Some(
                if input.expanded {
                    "**Expanded host guide**"
                } else {
                    "**Compact host guide**"
                }
                .into(),
            )
        })));
    let document = TuiDocument::new(ActivityDocument {
        title: "Host guide".into(),
        markdown: "Original **host** document\n\n```rust\nuse std::path::Path;\n```".into(),
    })
    .unwrap();
    assert!(apply_agent_poll(session.parts_mut().state, AgentPoll::Document(document)).unwrap());
    assert!(!session.parts_mut().state.turn_active());
    let source = session.session_output().unwrap().unwrap();
    assert!(source.contains("Original **host** document"));
    assert!(!source.contains("Compact host guide"));
    for (width, toggle, expanded) in [(80, false, false), (24, true, true), (80, false, true)] {
        if toggle {
            session
                .parts_mut()
                .state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::ALT),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 32), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Expanded host guide"), expanded, "{text}");
        assert_eq!(text.contains("Compact host guide"), !expanded, "{text}");
        assert!(!text.contains("Working"));
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 세션 문서 생성자는 기존 문서 wire 한도를 그대로 적용하고 첫 초과와 빈 제목을 거부한다.
#[test]
fn host_document_validates_before_publication() {
    use yo_core::{ActivityDocument, ToolOutput};

    use crate::TuiDocument;

    assert!(
        TuiDocument::new(ActivityDocument {
            title: String::new(),
            markdown: "body".into()
        })
        .is_none()
    );
    let mut document = ActivityDocument {
        title: "Guide".into(),
        markdown: String::new(),
    };
    let overhead = document.to_snapshot().unwrap().len();
    document.markdown = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
    assert!(TuiDocument::new(document.clone()).is_some());
    document.markdown.push('x');
    assert!(TuiDocument::new(document).is_none());
}

// 호스트의 초기 펼침은 전역 상태와 독립적으로 적용되지만 이후 사용자 토글은 항상 우선한다.
#[test]
fn host_document_initial_expansion_is_optional_and_user_overridable() {
    use std::time::Duration;

    use yo_core::ActivityDocument;

    use super::key;
    use crate::{
        DocumentRenderer, TuiDocument,
        input::event::{KeyCode, KeyModifiers},
        runner::{AgentPoll, unix::apply_agent_poll},
    };

    for global in [false, true] {
        for initial in [None, Some(false), Some(true)] {
            let mut session =
                TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
                    .with_document_renderer(Some(DocumentRenderer::new(|input| {
                        Some(
                            if input.expanded {
                                "Expanded guide"
                            } else {
                                "Compact guide"
                            }
                            .into(),
                        )
                    })));
            if global {
                session
                    .parts_mut()
                    .state
                    .handle(
                        key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                        Duration::ZERO,
                    )
                    .unwrap();
            }
            let document = TuiDocument::new(ActivityDocument {
                title: "Guide".into(),
                markdown: "Original source".into(),
            })
            .unwrap();
            let document = initial.map_or_else(
                || document.clone(),
                |expanded| document.clone().with_expanded(expanded),
            );
            apply_agent_poll(session.parts_mut().state, AgentPoll::Document(document)).unwrap();
            let original = session.session_output().unwrap();
            for (toggle, expected, width) in [
                (false, initial.unwrap_or(global), 80),
                (true, !initial.unwrap_or(global), 24),
                (false, !initial.unwrap_or(global), 80),
            ] {
                if toggle {
                    session
                        .parts_mut()
                        .state
                        .handle(
                            key(KeyCode::Character('o'), KeyModifiers::ALT),
                            Duration::ZERO,
                        )
                        .unwrap();
                }
                let pin = session.appearance_pin();
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 24), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                assert_eq!(
                    text.contains("Expanded guide"),
                    expected,
                    "{global}/{initial:?}: {text}"
                );
                assert_eq!(text.contains("Compact guide"), !expected, "{text}");
                session.parts_mut().state.commit_frame(&frame);
                assert_eq!(session.session_output().unwrap(), original);
            }
        }
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

// 답변 콜백은 본문·폭·완료 상태만 받고 실패 원문·다른 역할·내보내기를 바꾸지 않는다.
#[test]
fn assistant_renderer_preserves_outcomes_roles_source_and_reflow() {
    use std::sync::{Arc, Mutex};

    use yo_core::{ActivityOutcome, Failure};

    use crate::{AssistantRenderer, Theme};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&seen);
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_assistant_renderer(Some(AssistantRenderer::new(move |input| {
            captured.lock().unwrap().push((
                input.source.to_owned(),
                input.columns.get(),
                input.finalized,
            ));
            Some(format!(
                "## Adapted answer\n\n```rust\nlet ready = true;\n```\n\nWidth {}",
                input.columns
            ))
        })));
    let state = session.parts_mut().state;
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("USER SOURCE"),
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
            update: ActivityUpdate::TextSnapshot("Original **answer**".to_owned()),
        })
        .unwrap();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 40), &pin)
        .unwrap();
    assert!(visible_rows(&frame.surface).contains("Adapted answer"));
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, _, finalized)| !finalized)
    );
    session.parts_mut().state.commit_frame(&frame);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("ACTUAL **failure**")),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(2),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(2),
            update: ActivityUpdate::TextSnapshot("TOOL SOURCE".to_owned()),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(2),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let original = session.session_output().unwrap();
    for theme in [Theme::Default, Theme::Light, Theme::Mono] {
        session = session.with_theme(theme);
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            for expected in [
                "Adapted answer",
                "USER SOURCE",
                "TOOL SOURCE",
                "ACTUAL **failure**",
            ] {
                assert!(
                    text.split_whitespace()
                        .collect::<String>()
                        .contains(&expected.split_whitespace().collect::<String>()),
                    "{text}"
                );
            }
            assert!(text.contains(&format!("Width {}", width - 2)));
            session.parts_mut().state.commit_frame(&frame);
            assert_eq!(session.session_output().unwrap(), original);
        }
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(source, _, _)| source == "Original **answer**")
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, _, finalized)| *finalized)
    );
    for (renderer, expected) in [
        (
            Some(AssistantRenderer::new(|_| {
                Some("Second presentation".into())
            })),
            "Second presentation",
        ),
        (
            Some(AssistantRenderer::new(|_| Some("x\n\n".repeat(32768)))),
            "Original answer",
        ),
        (
            Some(AssistantRenderer::new(|_| {
                Some("x".repeat(AssistantRenderer::MAX_RENDERED_BYTES + 1))
            })),
            "Original answer",
        ),
        (Some(AssistantRenderer::new(|_| None)), "Original answer"),
        (None, "Original answer"),
        (
            Some(AssistantRenderer::new(|_| Some(String::new()))),
            "ACTUAL **failure**",
        ),
    ] {
        session = session.with_assistant_renderer(renderer);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains(&expected.split_whitespace().collect::<String>()),
            "{text}"
        );
        assert!(text.contains("ACTUAL **failure**"), "{text}");
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap(), original);
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

// 메시지 이미지도 공통 표시 설정·폭 변경·커스텀 렌더러를 따르며 원본과 실패 문구를 보존한다.
#[test]
fn message_content_images_reflow_customize_and_preserve_source() {
    use std::io::Cursor;

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use yo_core::{ActivityOutcome, Failure, MessageContent};

    use crate::{AssistantRenderer, OutputPreferences};

    let mut png = Cursor::new(Vec::new());
    RgbImage::new(4, 2)
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let block = json!({"type":"image","mimeType":"image/png","data":STANDARD.encode(png.get_ref()),"_meta":{"origin":"fixture"}});
    let source = MessageContent {
        block: block.clone(),
    }
    .to_snapshot()
    .unwrap();
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
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
            update: ActivityUpdate::TextSnapshot(source),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("delivery stopped")),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("fixture"));
    assert!(original.contains(block["data"].as_str().unwrap()));
    assert!(original.contains("delivery stopped"));
    assert!(!original.contains(MessageContent::SCHEMA));
    for visible in [true, false, true] {
        session =
            session.with_output_preferences(OutputPreferences::default().with_images(visible));
        for width in [80, 12, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 30), &pin)
                .unwrap();
            assert_eq!(!frame.surface.rasters.is_empty(), visible);
            if visible {
                assert_eq!(frame.surface.rasters[0].png.as_ref(), png.get_ref());
            }
            let rendered = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            assert!(rendered.contains("deliverystopped"), "{rendered}");
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
    let expected = block;
    session = session.with_assistant_renderer(Some(AssistantRenderer::new(move |input| {
        assert_eq!(
            MessageContent::from_snapshot(input.source).unwrap().block,
            expected
        );
        Some("Custom media answer".into())
    })));
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(visible_rows(&frame.surface).contains("Custom media answer"));
    assert!(frame.surface.rasters.is_empty());
    assert_eq!(session.session_output().unwrap().unwrap(), original);
}

// 답변 리소스·오디오·알 수 없는 블록은 공통 표시와 원본 보존을 함께 유지한다.
#[test]
fn message_content_resources_and_unknown_blocks_remain_observable() {
    use serde_json::json;
    use yo_core::{ActivityOutcome, MessageContent};
    for (block, marker) in [
        (
            json!({"type":"resource","resource":{"uri":"resource://main.rs","mimeType":"text/x-rust","text":"fn main() {}"}}),
            "fnmain(){}",
        ),
        (
            json!({"type":"resource_link","uri":"resource://guide","name":"Guide"}),
            "Guide",
        ),
        (
            json!({"type":"audio","mimeType":"audio/wav","data":"AA=="}),
            "playbackunavailable",
        ),
        (json!({"type":"future","payload":"RETAINED"}), "RETAINED"),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
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
                update: ActivityUpdate::TextSnapshot(
                    MessageContent {
                        block: block.clone(),
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 12, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            assert!(text.contains(marker), "{text}");
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        assert!(original.contains(block["type"].as_str().unwrap()));
    }
}

// 불투명한 검색 결과는 폭 변경 뒤에도 원문으로 표시하며 콜백·내보내기에 전체 값을 전달한다.
#[test]
fn opaque_web_results_remain_visible_and_customizable() {
    use serde_json::json;
    use yo_core::{ActivityOutcome, ToolOutput};

    use crate::{OutputPreferences, ToolRenderer};
    let results = json!([
        {"title":"![literal](data:image/png;base64,AAAA)","url":"https://example.com/source","future":"RETAINED"},
        {"type":"new-result","payload":"SECOND"},
    ]);
    let output = ToolOutput {
        tool: "webSearch".into(),
        server: None,
        arguments: Some(json!({"query":"sources"})),
        result: Some(json!({"results":results})),
        content_items: None,
        error: None,
        plain_text: format!("Web search\nQuery: sources\nResults:\n{results:#}"),
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
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
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("RETAINED") && original.contains("SECOND"));
    for width in [80, 24, 80] {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 70), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface)
            .split_whitespace()
            .collect::<String>();
        for token in [
            "RETAINED",
            "SECOND",
            "https://example.com/source",
            "![literal]",
        ] {
            assert!(text.contains(token), "{text}");
        }
        assert!(frame.surface.rasters.is_empty());
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
        assert_eq!(input.output, Some(&output));
        Some("Custom search results".into())
    })));
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(visible_rows(&frame.surface).contains("Custom search results"));
    assert_eq!(session.session_output().unwrap().unwrap(), original);
}

// 공급자가 전달한 추론은 공개 요약과 구분되며 숨김·폭 변경·커스텀 문서에도 원문과 실패를 보존한다.
#[test]
fn provider_reasoning_visibility_customization_and_export_preserve_source() {
    use yo_core::{ActivityOutcome, ActivityReasoning, Failure};

    use crate::{DocumentRenderer, OutputPreferences};
    for content in [
        serde_json::json!("**Original thought**"),
        serde_json::json!({"type":"future", "text":"![literal](data:image/png;base64,AA==)"}),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivityReasoning {
                        content: content.clone(),
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("Delivery stopped")),
            })
            .unwrap();
        let source = session.session_output().unwrap().unwrap();
        assert!(source.contains("Original thought") || source.contains("![literal]"));
        for custom in [false, true] {
            session = session.with_document_renderer(custom.then(|| {
                DocumentRenderer::new(|input| {
                    assert_eq!(input.document.title, "Agent reasoning");
                    assert!(
                        input.document.markdown.contains("Original thought")
                            || input.document.markdown.contains("![literal]")
                    );
                    Some("Custom reasoning".into())
                })
            }));
            for (columns, visible) in [(80, true), (24, false), (24, true), (80, false)] {
                session = session.with_output_preferences(
                    OutputPreferences::default()
                        .with_tool_head_rows(u16::MAX)
                        .with_reasoning(visible),
                );
                let pin = session.appearance_pin();
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(columns, 35), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                assert!(text.contains("Agent reasoning"), "{text}");
                assert!(!text.contains("Public reasoning summary"));
                assert_eq!(text.contains("Reasoning hidden"), !visible, "{text}");
                assert_eq!(
                    text.contains("Custom reasoning"),
                    visible && custom,
                    "{text}"
                );
                if !custom {
                    assert_eq!(
                        text.split_whitespace()
                            .collect::<String>()
                            .contains("Originalthought")
                            || text
                                .split_whitespace()
                                .collect::<String>()
                                .contains("![literal]"),
                        visible,
                        "{text}"
                    );
                }
                assert!(
                    text.split_whitespace()
                        .collect::<String>()
                        .contains("Deliverystopped"),
                    "{text}"
                );
                session.parts_mut().state.commit_frame(&frame);
                assert_eq!(session.session_output().unwrap().unwrap(), source);
            }
        }
    }
}

// 실행 호스트 링크 이벤트는 실제 appearance 경계에서 폭 재배치·매핑 교체·해제를 적용하고 원문을
// 유지한다.
#[test]
fn live_host_link_events_replace_and_clear_rendered_destinations() {
    use crate::{
        LinkResolver,
        runner::{AgentPoll, unix::apply_host_poll},
        surface::Hyperlink,
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
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
            update: ActivityUpdate::TextSnapshot("[Workspace file](file.rs)".into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for (width, path) in [
        (80, Some("/tmp/first.rs")),
        (24, Some("/tmp/second.rs")),
        (80, None),
    ] {
        let target = path.and_then(|path| Hyperlink::from_file_path(std::path::Path::new(path)));
        let expected = target.clone();
        let resolver = target.map(|target| {
            LinkResolver::new(move |destination| (destination == "file.rs").then(|| target.clone()))
        });
        let parts = session.parts_mut();
        assert!(
            apply_host_poll(parts.state, parts.appearance, AgentPoll::Links(resolver)).unwrap()
        );
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 20), &pin)
            .unwrap();
        let mut links = Vec::new();
        for y in 0..20 {
            for x in 0..width {
                if let Some(link) = frame.surface.cell(Point::new(x, y)).unwrap().hyperlink() {
                    links.push(link.clone());
                }
            }
        }
        if let Some(expected) = expected {
            assert!(!links.is_empty());
            assert!(links.iter().all(|link| link == &expected));
        } else {
            assert!(links.is_empty());
        }
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 공개 요약과 공급자 추론 모두 긴 본문을 접고 좁은 폭에서 펼쳐도 원문·끝부분·실패를 보존한다.
#[test]
fn reasoning_folding_preserves_tail_failure_and_source_after_resize() {
    use yo_core::{ActivityOutcome, ActivityReasoning, ActivitySummary, Failure, SummaryKind};

    use super::key;
    use crate::{
        OutputPreferences,
        input::event::{KeyCode, KeyModifiers},
    };
    let body = (0..20)
        .map(|line| format!("Reasoning line {line:02}\n\n"))
        .collect::<String>();
    let sources = [
        ActivitySummary {
            kind: SummaryKind::Reasoning,
            summary: body.clone(),
            tokens_before: None,
        }
        .to_snapshot()
        .unwrap(),
        ActivityReasoning {
            content: serde_json::json!(body),
        }
        .to_snapshot()
        .unwrap(),
    ];
    for source in sources {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(2));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
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
                outcome: ActivityOutcome::Failed(Failure::new("Retained failure")),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for (toggle, width, expanded) in [(false, 80, false), (true, 24, true), (true, 80, false)] {
            if toggle {
                session
                    .parts_mut()
                    .state
                    .handle(
                        key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                        Duration::ZERO,
                    )
                    .unwrap();
            }
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 60), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let flat = text.split_whitespace().collect::<String>();
            assert_eq!(flat.contains("rowshidden"), !expanded, "{text}");
            assert_eq!(flat.contains("Reasoningline08"), expanded, "{text}");
            assert!(flat.contains("Reasoningline19"), "{text}");
            assert!(flat.contains("Retainedfailure"), "{text}");
            session.parts_mut().state.commit_frame(&frame);
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
}
