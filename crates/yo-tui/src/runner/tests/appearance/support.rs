pub(super) use std::{path, time::Duration};

pub(super) use yo_core::{
    ActivityKind, ActivityUpdate, AgentCommand, AgentEvent, TranscriptRecord, UserInput,
};

pub(super) use super::super::{activity, turn};
pub(super) use crate::{
    PresentationMode, TuiSessionInfo, appearance,
    appearance::{
        AppearanceCandidate, AppearanceState, ColorCapability, GlyphProfile, MotionPreference,
    },
    html::HtmlSurface,
    overlay,
    prompt::{PromptGlyphs, PromptStyles},
    runner::{session::TuiSession, state::TuiState},
    shell::{AgentShellStyles, ShellChromeStyles},
    surface::{Attributes, CellContent, Color, FrameDiff, Point, Size, Style, Surface},
    terminal::{TerminalOp, TerminalOps},
    transcript::{MarkdownStyles, TranscriptActivityStyles, TranscriptStyles},
};

pub(super) const FRAME_SIZE: Size = Size::new(20, 11);

pub(super) fn conversation() -> TuiState {
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

pub(super) fn populate_session(session: &mut TuiSession) {
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

pub(super) fn marker(surface: &Surface, row: u16) -> (&str, u16, Style) {
    let cell = surface.cell(Point::new(0, row)).unwrap();
    let CellContent::Grapheme { text, width } = cell.content() else {
        panic!("the transcript row must begin with a marker");
    };
    (text, width.get(), cell.style())
}

pub(super) fn grapheme_at(surface: &Surface, point: Point) -> &str {
    let CellContent::Grapheme { text, .. } = surface.cell(point).unwrap().content() else {
        panic!("the selected cell must contain a grapheme");
    };
    text
}

pub(super) fn visible_rows(surface: &Surface) -> String {
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
