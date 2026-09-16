use yo_core::ImagePreparationHost;

use super::{
    super::{
        FrameRateLimit, PendingDispatch, PresentationMode, SkillReferenceConnection,
        WorkspaceReferenceConnection, state::TuiState,
    },
    TuiSession,
};
use crate::{appearance::AppearanceState, terminal, transcript::TranscriptMeasureError};

/// 이 Session에서 관찰한 터미널 게시 보정 한 건입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PublicationRecoveryKind {
    /// 주소를 확인할 수 있는 접두사를 지우고 게시를 다시 시작했습니다.
    ReversibleRestart,
    /// 검증된 네이티브 이력 접두사를 보존하고 정확한 접미사부터 재개했습니다.
    IrreversibleResume,
    /// 버퍼링되지 않은 전송 flush만 다시 시도했습니다.
    FlushRetry,
}

/// 복구된 터미널 게시 오류에 대한 제한된 환경 근거입니다.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublicationRecoveryEvidence {
    reversible_restarts: u64,
    irreversible_resumes: u64,
    flush_retries: u64,
    last: Option<PublicationRecoveryKind>,
}

pub(in crate::runner) struct SessionParts<'session> {
    pub(in crate::runner) state: &'session mut TuiState,
    pub(in crate::runner) appearance: &'session mut AppearanceState,
    pub(in crate::runner) pending_dispatch: &'session mut Option<PendingDispatch>,
    pub(in crate::runner) pending_control: &'session mut Option<PendingDispatch>,
    pub(in crate::runner) frame_rate_limit: FrameRateLimit,
    pub(in crate::runner) workspace_references:
        &'session mut Option<Box<dyn WorkspaceReferenceConnection>>,
    pub(in crate::runner) skill_references: &'session mut Option<Box<dyn SkillReferenceConnection>>,
    pub(in crate::runner) image_preparation: &'session mut Option<Box<dyn ImagePreparationHost>>,
    pub(in crate::runner) publication_recovery_evidence: &'session mut PublicationRecoveryEvidence,
}
impl TuiSession {
    pub(in crate::runner) fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.state.set_presentation_mode(mode);
    }

    pub(in crate::runner) fn session_output(
        &self,
    ) -> Result<Option<String>, TranscriptMeasureError> {
        self.state.session_output(&self.appearance.pin())
    }

    /// 환경 근거로 보관한 게시 오류 복구 정보를 반환합니다.
    #[must_use]
    pub const fn publication_recovery_evidence(&self) -> PublicationRecoveryEvidence {
        self.publication_recovery_evidence
    }

    pub(in crate::runner) fn parts_mut(&mut self) -> SessionParts<'_> {
        SessionParts {
            state: &mut self.state,
            appearance: &mut self.appearance,
            pending_dispatch: &mut self.pending_dispatch,
            pending_control: &mut self.pending_control,
            frame_rate_limit: self.frame_rate_limit,
            workspace_references: &mut self.workspace_references,
            skill_references: &mut self.skill_references,
            image_preparation: &mut self.image_preparation,
            publication_recovery_evidence: &mut self.publication_recovery_evidence,
        }
    }
}
impl PublicationRecoveryEvidence {
    /// 지우고 게시를 다시 시작한 주소 확인 가능 접두사의 개수를 반환합니다.
    #[must_use]
    pub const fn reversible_restarts(self) -> u64 {
        self.reversible_restarts
    }

    /// 검증된 네이티브 이력 접두사에서 정확한 접미사부터 재개한 횟수를 반환합니다.
    #[must_use]
    pub const fn irreversible_resumes(self) -> u64 {
        self.irreversible_resumes
    }

    /// 바이트를 재생하지 않고 버퍼링되지 않은 flush를 다시 시도한 횟수를 반환합니다.
    #[must_use]
    pub const fn flush_retries(self) -> u64 {
        self.flush_retries
    }

    /// 가장 최근에 관찰한 보정을 반환하며, 없으면 None을 반환합니다.
    #[must_use]
    pub const fn last(self) -> Option<PublicationRecoveryKind> {
        self.last
    }

    pub(in crate::runner) fn record(&mut self, recovery: terminal::mode::inline::InlineRecovery) {
        let (counter, kind) = match recovery {
            terminal::mode::inline::InlineRecovery::ReversibleRestart => (
                &mut self.reversible_restarts,
                PublicationRecoveryKind::ReversibleRestart,
            ),
            terminal::mode::inline::InlineRecovery::IrreversibleResume => (
                &mut self.irreversible_resumes,
                PublicationRecoveryKind::IrreversibleResume,
            ),
            terminal::mode::inline::InlineRecovery::FlushRetry => {
                (&mut self.flush_retries, PublicationRecoveryKind::FlushRetry)
            },
        };
        *counter = counter.saturating_add(1);
        self.last = Some(kind);
    }
}
