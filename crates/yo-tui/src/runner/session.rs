use super::{PendingDispatch, state::TuiState};

/// Terminal-independent state retained across terminal ownership generations.
///
/// A process host can release and reacquire the terminal while keeping the
/// same session value alive. Terminal modes, presenters, and frame history are
/// deliberately not stored here.
#[derive(Debug)]
pub struct TuiSession {
    state: TuiState,
    pending_dispatch: Option<PendingDispatch>,
    pending_control: Option<PendingDispatch>,
}

pub(super) struct SessionParts<'session> {
    pub(super) state: &'session mut TuiState,
    pub(super) pending_dispatch: &'session mut Option<PendingDispatch>,
    pub(super) pending_control: &'session mut Option<PendingDispatch>,
}

impl TuiSession {
    /// Creates an empty TUI session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: TuiState::new(),
            pending_dispatch: None,
            pending_control: None,
        }
    }

    pub(super) const fn state(&self) -> &TuiState {
        &self.state
    }

    pub(super) fn parts_mut(&mut self) -> SessionParts<'_> {
        SessionParts {
            state: &mut self.state,
            pending_dispatch: &mut self.pending_dispatch,
            pending_control: &mut self.pending_control,
        }
    }
}

impl Default for TuiSession {
    fn default() -> Self {
        Self::new()
    }
}
