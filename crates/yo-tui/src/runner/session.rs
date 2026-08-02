use super::{PendingDispatch, PresentationMode, state::TuiState};
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

/// Host-known labels displayed in the TUI status line.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiSessionInfo {
    backend: Option<String>,
    workspace: String,
}

pub(super) struct SessionParts<'session> {
    pub(super) state: &'session mut TuiState,
    pub(super) appearance: &'session AppearanceState,
    pub(super) pending_dispatch: &'session mut Option<PendingDispatch>,
    pub(super) pending_control: &'session mut Option<PendingDispatch>,
}

impl TuiSession {
    /// Creates an empty TUI session with the compatibility-default Rich glyphs.
    #[must_use]
    pub fn new() -> Self {
        Self::with_glyph_profile(GlyphProfile::Rich)
    }

    /// Creates an empty TUI session with an explicit built-in glyph profile.
    #[must_use]
    pub fn with_glyph_profile(profile: GlyphProfile) -> Self {
        Self::with_session_info(profile, TuiSessionInfo::default())
    }

    /// Creates a session with host-known status labels and an explicit glyph profile.
    #[must_use]
    pub fn with_session_info(profile: GlyphProfile, info: TuiSessionInfo) -> Self {
        Self {
            state: TuiState::with_session_info(info),
            appearance: AppearanceState::new(AppearanceCandidate::for_profile(profile))
                .expect("built-in appearance profiles must always be valid"),
            pending_dispatch: None,
            pending_control: None,
        }
    }

    pub(super) fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.state.set_presentation_mode(mode);
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

    #[cfg(test)]
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

impl TuiSessionInfo {
    /// Creates safe, single-line status labels from host-provided display values.
    #[must_use]
    pub fn new(backend: impl Into<String>, workspace: impl Into<String>) -> Self {
        Self {
            backend: non_empty_label(backend.into()),
            workspace: single_line_label(workspace.into()),
        }
    }

    pub(super) fn backend(&self) -> Option<&str> {
        self.backend.as_deref()
    }

    pub(super) fn workspace(&self) -> &str {
        &self.workspace
    }
}

fn non_empty_label(value: String) -> Option<String> {
    let label = single_line_label(value);
    (!label.is_empty()).then_some(label)
}

fn single_line_label(value: String) -> String {
    let mut label = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            label.extend(character.escape_default());
        } else {
            label.push(character);
        }
    }
    label
}

impl Default for TuiSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::TuiSessionInfo;

    // 외부 backend가 전달한 제어 문자는 status line의 행 구조를 바꾸지 못하고 보이는 표기로 바뀐다.
    #[test]
    fn session_info_escapes_control_characters_into_one_line() {
        let info = TuiSessionInfo::new("co\ndex", "work\tspace");

        assert_eq!(info.backend(), Some("co\\ndex"));
        assert_eq!(info.workspace(), "work\\tspace");
    }
}
