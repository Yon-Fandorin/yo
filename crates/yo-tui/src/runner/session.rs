mod bridges;
mod construction;
mod continuation;
mod external_editor;
mod generation;
mod metadata;

pub use external_editor::{ExternalEditorImportError, ExternalEditorSnapshot};
pub(in crate::runner) use generation::SessionParts;
pub use generation::{PublicationRecoveryEvidence, PublicationRecoveryKind};
pub use metadata::{
    ResumeSessionEntry, TuiDocument, TuiSessionInfo, TuiStatusError, TuiStatusLine,
};
use yo_core::ImagePreparationHost;

use super::{
    FrameRateLimit, PendingDispatch, SkillReferenceConnection, WorkspaceReferenceConnection,
    state::TuiState,
};
use crate::appearance::AppearanceState;

/// 터미널 소유 세대가 바뀌어도 유지되는 터미널 비종속 상태.
///
/// 프로세스 호스트는 같은 세션 값을 유지한 채 터미널을 반납했다가 다시
/// 획득할 수 있습니다. 터미널 모드·프레젠터·프레임 이력은 이곳에 저장하지
/// 않습니다.
pub struct TuiSession {
    state: TuiState,
    appearance: AppearanceState,
    pending_dispatch: Option<PendingDispatch>,
    pending_control: Option<PendingDispatch>,
    frame_rate_limit: FrameRateLimit,
    workspace_references: Option<Box<dyn WorkspaceReferenceConnection>>,
    skill_references: Option<Box<dyn SkillReferenceConnection>>,
    image_preparation: Option<Box<dyn ImagePreparationHost>>,
    publication_recovery_evidence: PublicationRecoveryEvidence,
}

#[cfg(test)]
mod tests;
