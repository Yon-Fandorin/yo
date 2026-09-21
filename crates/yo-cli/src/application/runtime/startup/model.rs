use std::path::PathBuf;

use yo_core::{
    ImagePreparationHost, SkillReferenceProvider,
    session_repository::{InheritedSessionHistory, StoredSessionContinuation},
};

use crate::{
    application::{agent, codex_diagnostics::CodexWarningCollector},
    execution::{model as execution_model, tools as local_tools},
    state::config,
};

/// 시작 결과를 소비할 프론트엔드 종류입니다.
#[derive(Clone, Copy)]
pub(in crate::application::runtime) enum StartupFrontend {
    Terminal,
    Print,
}

/// 하나의 live generation에서 공유하는 설정과 자격 증명 snapshot입니다.
pub(in crate::application::runtime) struct StartupSnapshots<'a> {
    pub(in crate::application::runtime) config: &'a config::Config,
    pub(in crate::application::runtime) credentials: &'a mut Option<yo_core::CredentialSnapshot>,
    pub(in crate::application::runtime) stored_preference: Option<&'a yo_core::StartupTarget>,
    pub(in crate::application::runtime) codex_warnings: &'a CodexWarningCollector,
}

/// 프론트엔드가 소비할 backend와 Session 자원 묶음입니다.
pub(in crate::application::runtime) struct PreparedAgent {
    pub(in crate::application::runtime) is_resume: bool,
    pub(in crate::application::runtime) session_id: yo_core::SessionId,
    pub(in crate::application::runtime) inherited_history: Option<InheritedSessionHistory>,
    pub(in crate::application::runtime) restored_prompt_history:
        Option<yo_tui::RestoredPromptHistory>,
    pub(in crate::application::runtime) notification_history_cutoff: Option<yo_core::TurnRef>,
    pub(in crate::application::runtime) agent: agent::TuiAgentConnection,
    pub(in crate::application::runtime) workspace: PathBuf,
    pub(in crate::application::runtime) workspace_references:
        Option<yo_core::LocalWorkspaceReferenceProvider>,
    pub(in crate::application::runtime) skill_references: Option<Box<dyn SkillReferenceProvider>>,
    pub(in crate::application::runtime) image_preparation: Box<dyn ImagePreparationHost>,
    pub(in crate::application::runtime) selection: execution_model::StartupBackend,
    pub(in crate::application::runtime) local_tool_registry:
        Option<local_tools::LocalToolRegistryRevision>,
    pub(in crate::application::runtime) execution_manifest_digest: Option<String>,
    pub(in crate::application::runtime) active_host: Option<yo_core::HostId>,
    pub(in crate::application::runtime) active_host_execution:
        Option<execution_model::DelegatedExecutionProfile>,
    pub(in crate::application::runtime) active_host_model: Option<execution_model::ActiveHostModel>,
    pub(in crate::application::runtime) host_catalogs: Vec<execution_model::HostCatalogObservation>,
}

/// Startup이 준비한 Session의 최종 상태입니다.
pub(in crate::application::runtime) enum StartupOutcome {
    Ready(Box<PreparedAgent>),
    Complete,
}

/// 새 Session, 재개 Session, fork Session의 조립 입력입니다.
pub(super) enum Launch {
    New(yo_core::SessionDescriptor),
    Resume(Box<StoredSessionContinuation>),
    Fork {
        descriptor: yo_core::SessionDescriptor,
        parent: Box<StoredSessionContinuation>,
    },
}

/// 일반 실행과 fork 실행을 구분하는 startup 요청입니다.
pub(super) enum LaunchRequest<'a> {
    Options(Option<&'a execution_model::ActiveHostModel>),
    Fork(Box<StoredSessionContinuation>),
}

impl Launch {
    pub(super) fn resume_id(&self) -> Option<yo_core::SessionId> {
        match self {
            Self::New(_) | Self::Fork { .. } => None,
            Self::Resume(continuation) => Some(continuation.descriptor().session_id()),
        }
    }
}
