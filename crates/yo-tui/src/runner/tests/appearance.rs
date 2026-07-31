use yo_core::{
    ActivityKind, ActivityUpdate, AgentCommand, AgentEvent, TranscriptRecord, UserInput,
};

use super::{TuiSession, TuiState, activity, turn};
use crate::{
    appearance::{AppearanceCandidate, AppearanceState, GlyphProfile},
    html::HtmlSurface,
    prompt::{PromptGlyphs, PromptStyles},
    shell::AgentShellStyles,
    surface::{Attributes, CellContent, Color, FrameDiff, Point, Size, Style, Surface},
    terminal::{TerminalOp, TerminalOps},
    transcript::TranscriptStyles,
};

const FRAME_SIZE: Size = Size::new(20, 5);

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
    assert_eq!(marker(&frame.surface, 2).0, "⏺");

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

    assert_eq!(marker(&rich.surface, 6), ("─", 1, rule_style));
    assert_eq!(marker(&rich.surface, 7), ("›", 1, marker_style));
    assert_eq!(marker(&rich.surface, 8), ("─", 1, rule_style));
    assert_eq!(marker(&ascii.surface, 6), ("-", 1, rule_style));
    assert_eq!(marker(&ascii.surface, 7), (">", 1, marker_style));
    assert_eq!(marker(&ascii.surface, 8), ("-", 1, rule_style));
    assert_eq!(rich.surface.cell(Point::new(2, 7)).unwrap().style(), body);
    assert_eq!(ascii.surface.cell(Point::new(2, 7)).unwrap().style(), body);
}

// ASCII snapshot은 화면 transcript와 빈 입력 marker를 함께 ASCII로 그리고, 종료용 plain
// output은 같은 snapshot의 transcript만 내보내 profile 일관성과 출력 경계를 함께 지킨다.
#[test]
fn screen_and_session_output_share_the_same_committed_snapshot() {
    let state = conversation();
    let appearance =
        AppearanceState::new(AppearanceCandidate::for_profile(GlyphProfile::Ascii)).unwrap();
    let pin = appearance.pin();
    let frame = state.prepare_frame(FRAME_SIZE, &pin).unwrap();
    let output = state.session_output(&pin).unwrap().unwrap();

    assert_eq!(visible_rows(&frame.surface), "> question\n\n* answer\n\n>");
    assert_eq!(output, "> question\n\n* answer\n");
}

// 한 TuiSession의 profile 교체는 다른 세션의 snapshot과 revision에 전파되지 않는다.
#[test]
fn appearance_replacement_is_isolated_per_session() {
    let mut first = TuiSession::new();
    let mut second = TuiSession::new();
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
        },
        prompt: PromptStyles {
            body: default,
            marker: default,
            rule: default,
            glyphs: PromptGlyphs::ascii(),
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
