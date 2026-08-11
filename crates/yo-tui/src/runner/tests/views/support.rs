use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentCommand, AgentEvent, TranscriptRecord,
    UserInput,
};

use super::super::{activity, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    runner::state::TuiState,
    surface::{CellContent, Point, Size, Surface},
};

fn rendered(surface: &Surface) -> String {
    let size = surface.size();
    (0..size.height)
        .map(|y| {
            (0..size.width)
                .map(
                    |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank | CellContent::Continuation { .. } => ' ',
                        CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
                    },
                )
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn render_and_commit(state: &mut TuiState, size: Size) -> String {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    let output = rendered(&frame.surface);
    state.commit_frame(&frame);
    output
}

pub(super) fn function(number: u8, action: KeyAction) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code: KeyCode::Function(number),
        modifiers: KeyModifiers::NONE,
        action,
        state: KeyState::NONE,
    })
}

pub(super) fn observed_conversation() -> TuiState {
    let mut state = TuiState::new();
    let tool = activity(1);
    for record in [
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("inspect the repository"),
        }),
        TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn: turn() }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity: tool,
            update: ActivityUpdate::TextSnapshot("cargo test -p yo-tui".to_owned()),
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity: tool,
            outcome: ActivityOutcome::Completed,
        }),
    ] {
        state.observe_record(record).unwrap();
    }
    state
}
