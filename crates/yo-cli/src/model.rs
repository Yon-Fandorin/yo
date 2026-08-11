//! Startup model binding resolution and host-owned native backend services.

use std::path::Path;

#[cfg(test)]
use yo_core::ModelSelection;
use yo_core::{AccountId, AgentBackend, BackendResumeTarget, CredentialStore, ModelId, ProviderId};

use crate::{AppError, config::Config};

mod native;
mod startup;
mod tokenizer;

#[derive(Clone, Debug)]
pub(crate) enum StartupBackend {
    Codex,
    Native {
        provider: ProviderId,
        account: AccountId,
        model: ModelId,
        replace_binding: bool,
    },
}

impl StartupBackend {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Codex => "codex",
            Self::Native { model, .. } => model.as_str(),
        }
    }

    pub(crate) const fn replaces_binding(&self) -> bool {
        matches!(
            self,
            Self::Native {
                replace_binding: true,
                ..
            }
        )
    }

    pub(crate) fn model_selection(&self) -> Option<yo_core::ModelSelection> {
        match self {
            Self::Codex => None,
            Self::Native {
                provider,
                account,
                model,
                ..
            } => Some(yo_core::ModelSelection::new(
                provider.clone(),
                account.clone(),
                model.clone(),
            )),
        }
    }
}

pub(crate) fn replacement(selection: &yo_core::ModelSelection) -> StartupBackend {
    startup::replacement(selection)
}

pub(crate) fn resolve(
    config: &Config,
    override_model: Option<&str>,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    startup::resolve(config, override_model, resume)
}

pub(crate) fn start_native(
    config: &Config,
    credentials: &CredentialStore,
    selection: &StartupBackend,
    workspace: &Path,
) -> Result<Box<dyn AgentBackend + Send>, AppError> {
    native::start_native(config, credentials, selection, workspace)
}

pub(crate) fn open_credentials(path: &Path) -> Result<CredentialStore, AppError> {
    native::open_credentials(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // backend facade는 host와 native selection의 label·좌표·replacement flag를 각각
    // 보존하고, replacement helper도 같은 좌표를 durable binding 교체로 표시한다.
    #[test]
    fn startup_backend_metadata_preserves_host_and_native_selection_semantics() {
        let selection = ModelSelection::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("token-plan").unwrap(),
            ModelId::new("qwen3.8max").unwrap(),
        );

        let codex = StartupBackend::Codex;
        assert_eq!(codex.label(), "codex");
        assert!(!codex.replaces_binding());
        assert!(codex.model_selection().is_none());

        let native = StartupBackend::Native {
            provider: selection.provider().clone(),
            account: selection.account().clone(),
            model: selection.model().clone(),
            replace_binding: false,
        };
        assert_eq!(native.label(), "qwen3.8max");
        assert!(!native.replaces_binding());
        assert_eq!(native.model_selection(), Some(selection.clone()));

        let replacement = replacement(&selection);
        assert_eq!(replacement.label(), "qwen3.8max");
        assert!(replacement.replaces_binding());
        assert_eq!(replacement.model_selection(), Some(selection));
    }
}
