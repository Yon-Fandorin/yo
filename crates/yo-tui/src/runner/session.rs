use super::{PendingDispatch, state::TuiState};
#[cfg(test)]
use crate::appearance::AppearancePin;
use crate::{
    appearance::{
        AppearanceCandidate, AppearanceCommitError, AppearanceRevision, AppearanceState,
        GlyphProfile,
    },
    transcript::TranscriptMeasureError,
};

/// Terminal-independent state retained across terminal ownership generations.
///
/// A process host can release and reacquire the terminal while keeping the
/// same session value alive. Terminal modes, presenters, and frame history are
/// deliberately not stored here.
#[derive(Debug)]
pub struct TuiSession {
    state: TuiState,
    appearance: AppearanceState,
    pending_dispatch: Option<PendingDispatch>,
    pending_control: Option<PendingDispatch>,
}

pub(super) struct SessionParts<'session> {
    pub(super) state: &'session mut TuiState,
    pub(super) appearance: &'session AppearanceState,
    pub(super) pending_dispatch: &'session mut Option<PendingDispatch>,
    pub(super) pending_control: &'session mut Option<PendingDispatch>,
}

impl TuiSession {
    /// Creates an empty TUI session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: TuiState::new(),
            appearance: AppearanceState::default(),
            pending_dispatch: None,
            pending_control: None,
        }
    }

    pub(super) fn session_output(&self) -> Result<Option<String>, TranscriptMeasureError> {
        self.state.session_output(&self.appearance.pin())
    }

    pub(super) fn parts_mut(&mut self) -> SessionParts<'_> {
        SessionParts {
            state: &mut self.state,
            appearance: &self.appearance,
            pending_dispatch: &mut self.pending_dispatch,
            pending_control: &mut self.pending_control,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the first Slice reserves a crate-private runtime replacement seam"
        )
    )]
    pub(crate) fn commit_appearance(
        &mut self,
        candidate: AppearanceCandidate,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        self.appearance.commit(candidate)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the first Slice reserves explicit ASCII selection without a public facade"
        )
    )]
    pub(crate) fn select_glyph_profile(
        &mut self,
        profile: GlyphProfile,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        self.commit_appearance(AppearanceCandidate::for_profile(profile))
    }

    #[cfg(test)]
    pub(super) fn appearance_pin(&self) -> AppearancePin {
        self.appearance.pin()
    }
}

impl Default for TuiSession {
    fn default() -> Self {
        Self::new()
    }
}
