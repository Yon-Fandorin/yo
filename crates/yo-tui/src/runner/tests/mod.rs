use std::num::NonZeroU64;

use yo_core::{ActivityId, ActivityRef, TurnId, TurnRef};

use super::state::TuiState;
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent as YoKeyEvent, KeyState},
    surface::{CellContent, Point, Size},
};

mod activity_projection;
mod admission;
mod appearance;
mod backpressure;
mod command_palette;
mod integration;
mod interrupt;
mod job_control;
mod model_selection;
mod overlay;
mod publication;
mod reentry;
mod request_responses;
mod session_lifecycle;
mod source_scheduling;
mod state_edges;
mod viewport;
mod views;

fn key(code: KeyCode, modifiers: crate::input::event::KeyModifiers) -> InputEvent {
    InputEvent::Key(YoKeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

fn rendered_row(state: &TuiState, size: Size, y: u16) -> String {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    (0..size.width)
        .map(
            |x| match frame.surface.cell(Point::new(x, y)).unwrap().content() {
                CellContent::Blank | CellContent::Continuation { .. } => ' ',
                CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn turn() -> TurnRef {
    let session_id = "01890f00-0000-7000-8000-000000000001"
        .parse()
        .expect("the fixture is a UUIDv7");
    TurnRef::new(session_id, TurnId::new(nonzero(1)))
}

fn activity(value: u64) -> ActivityRef {
    ActivityRef::new(turn(), ActivityId::new(nonzero(value)))
}
