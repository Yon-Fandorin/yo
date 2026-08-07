use super::{
    FrameRateLimit, PendingDispatch, PresentationMode, SkillReferenceConnection,
    WorkspaceReferenceConnection, state::TuiState,
};
#[cfg(test)]
use crate::appearance::AppearancePin;
use crate::{
    appearance::{
        AppearanceCandidate, AppearanceCommitError, AppearanceRevision, AppearanceState,
        ColorCapability, GlyphProfile, MotionPreference,
    },
    overlay::{AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, SlotError},
    transcript::TranscriptMeasureError,
};

/// Terminal-independent state retained across terminal ownership generations.
///
/// A process host can release and reacquire the terminal while keeping the
/// same session value alive. Terminal modes, presenters, and frame history are
/// deliberately not stored here.
pub struct TuiSession {
    state: TuiState,
    appearance: AppearanceState,
    pending_dispatch: Option<PendingDispatch>,
    pending_control: Option<PendingDispatch>,
    frame_rate_limit: FrameRateLimit,
    workspace_references: Option<Box<dyn WorkspaceReferenceConnection>>,
    skill_references: Option<Box<dyn SkillReferenceConnection>>,
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
    pub(super) frame_rate_limit: FrameRateLimit,
    pub(super) workspace_references: &'session mut Option<Box<dyn WorkspaceReferenceConnection>>,
    pub(super) skill_references: &'session mut Option<Box<dyn SkillReferenceConnection>>,
}

impl TuiSession {
    /// Creates an empty Rich-glyph TUI session from explicit host appearance facts.
    #[must_use]
    pub fn new(color_capability: ColorCapability, motion_preference: MotionPreference) -> Self {
        Self::with_glyph_profile(GlyphProfile::Rich, color_capability, motion_preference)
    }

    /// Creates an empty TUI session from an explicit profile and host appearance facts.
    #[must_use]
    pub fn with_glyph_profile(
        profile: GlyphProfile,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self::with_session_info(
            profile,
            TuiSessionInfo::default(),
            color_capability,
            motion_preference,
        )
    }

    /// Creates a session with explicit host labels, glyphs, color, and motion preference.
    #[must_use]
    pub fn with_session_info(
        profile: GlyphProfile,
        info: TuiSessionInfo,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self {
            state: TuiState::with_session_info(info),
            appearance: AppearanceState::new(
                AppearanceCandidate::for_profile_with_host_preferences(
                    profile,
                    color_capability,
                    motion_preference,
                ),
            )
            .expect("built-in appearance profiles must always be valid"),
            pending_dispatch: None,
            pending_control: None,
            frame_rate_limit: FrameRateLimit::default(),
            workspace_references: None,
            skill_references: None,
        }
    }

    /// Selects the maximum presentation rate used to coalesce live frame requests.
    #[must_use]
    pub fn with_frame_rate_limit(mut self, limit: FrameRateLimit) -> Self {
        self.frame_rate_limit = limit;
        self
    }

    /// Installs the execution environment's nonblocking workspace provider.
    #[must_use]
    pub fn with_workspace_references(
        mut self,
        connection: impl WorkspaceReferenceConnection + 'static,
    ) -> Self {
        self.workspace_references = Some(Box::new(connection));
        self.state.enable_workspace_references();
        self
    }

    /// Installs the execution environment's nonblocking skill catalog provider.
    #[must_use]
    pub fn with_skill_references(
        mut self,
        connection: impl SkillReferenceConnection + 'static,
    ) -> Self {
        self.skill_references = Some(Box::new(connection));
        self.state.enable_skill_references();
        self
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
            frame_rate_limit: self.frame_rate_limit,
            workspace_references: &mut self.workspace_references,
            skill_references: &mut self.skill_references,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn open_prompt_overlay(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.state.open_overlay(snapshot)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn refresh_prompt_overlay(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        self.state.refresh_overlay(token, snapshot)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn close_prompt_overlay(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<(), SlotError> {
        self.state.close_overlay(token)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn take_prompt_overlay_acceptance(&mut self) -> Option<AcceptanceReceipt> {
        self.state.take_overlay_acceptance()
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

#[cfg(test)]
mod tests {
    use super::{FrameRateLimit, TuiSession, TuiSessionInfo};
    use crate::{
        appearance::{ColorCapability, MotionPreference},
        overlay::{PanelSnapshot, SelectionEntry, SlotError},
    };

    fn panel(label: &str) -> PanelSnapshot {
        PanelSnapshot::new(
            "Commands",
            vec![SelectionEntry::enabled("entry", label, None)],
        )
        .unwrap()
    }

    // 외부 backend가 전달한 제어 문자는 status line의 행 구조를 바꾸지 못하고 보이는 표기로 바뀐다.
    #[test]
    fn session_info_escapes_control_characters_into_one_line() {
        let info = TuiSessionInfo::new("co\ndex", "work\tspace");

        assert_eq!(info.backend(), Some("co\\ndex"));
        assert_eq!(info.workspace(), "work\\tspace");
    }

    // TuiSession facade는 provider가 발급받은 token을 state slot에 그대로 전달하고,
    // close 뒤 같은 token의 refresh를 stale로 거절한다.
    #[test]
    fn session_facade_preserves_overlay_token_scope() {
        let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
        let token = session.open_prompt_overlay(panel("First")).unwrap();

        session
            .refresh_prompt_overlay(token, panel("Updated"))
            .unwrap();
        session.close_prompt_overlay(token).unwrap();
        assert_eq!(session.take_prompt_overlay_acceptance(), None);

        assert_eq!(
            session.refresh_prompt_overlay(token, panel("Late")),
            Err(SlotError::StaleToken)
        );
    }

    // 시작 시 선택한 frame 제한은 terminal ownership generation이 parts를 다시 빌려도 유지됩니다.
    #[test]
    fn frame_rate_limit_is_retained_across_generation_borrows() {
        let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard)
            .with_frame_rate_limit(FrameRateLimit::Fps60);

        assert_eq!(session.parts_mut().frame_rate_limit, FrameRateLimit::Fps60);
        assert_eq!(session.parts_mut().frame_rate_limit, FrameRateLimit::Fps60);
    }
}
