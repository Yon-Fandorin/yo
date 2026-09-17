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
pub(crate) enum StartupFrontend {
    Terminal,
    Print,
}

/// 하나의 live generation에서 공유하는 설정과 자격 증명 snapshot입니다.
pub(crate) struct StartupSnapshots<'a> {
    pub(crate) config: &'a config::Config,
    pub(crate) credentials: &'a mut Option<yo_core::CredentialSnapshot>,
    pub(crate) stored_preference: Option<&'a yo_core::StartupTarget>,
    pub(crate) codex_warnings: &'a CodexWarningCollector,
}

/// 프론트엔드가 소비할 backend와 Session 자원 묶음입니다.
pub(crate) struct PreparedAgent {
    pub(crate) is_resume: bool,
    pub(crate) session_id: yo_core::SessionId,
    pub(crate) inherited_history: Option<InheritedSessionHistory>,
    pub(crate) agent: agent::TuiAgentConnection,
    pub(crate) workspace: PathBuf,
    pub(crate) workspace_references: Option<yo_core::LocalWorkspaceReferenceProvider>,
    pub(crate) skill_references: Option<Box<dyn SkillReferenceProvider>>,
    pub(crate) image_preparation: Box<dyn ImagePreparationHost>,
    pub(crate) selection: execution_model::StartupBackend,
    pub(crate) local_tool_registry: Option<local_tools::LocalToolRegistryRevision>,
    pub(crate) execution_manifest_digest: Option<String>,
    pub(crate) active_host: Option<yo_core::HostId>,
    pub(crate) active_host_execution: Option<execution_model::DelegatedExecutionProfile>,
    pub(crate) active_host_model: Option<execution_model::ActiveHostModel>,
    pub(crate) host_catalogs: Vec<execution_model::HostCatalogObservation>,
}

/// Startup이 준비한 Session의 최종 상태입니다.
pub(crate) enum StartupOutcome {
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
