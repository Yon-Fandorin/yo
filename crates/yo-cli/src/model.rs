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
    stored_preference: Option<yo_core::StartupTarget>,
    override_model: Option<&str>,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    startup::resolve(config, stored_preference, override_model, resume)
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

pub(crate) fn credentials_for_startup<'a>(
    config: &Config,
    retained: &'a mut Option<CredentialStore>,
    selection: &StartupBackend,
) -> Result<Option<&'a CredentialStore>, AppError> {
    if matches!(selection, StartupBackend::Codex) {
        return Ok(None);
    }
    if retained.is_none() {
        *retained = Some(open_credentials(&config.credential_path())?);
    }
    Ok(retained.as_ref())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

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

    // Local Codex 선택은 옆의 credentials.yaml이 잘못되어도 그 파일을 required source로
    // 열지 않고, native 선택에서만 같은 파일 오류가 startup을 막는지 확인합니다.
    #[test]
    fn startup_opens_credentials_only_for_a_native_selection() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-credential-selection-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        let config_path = path.join("config.yaml");
        fs::write(&config_path, "session: {}\n").unwrap();
        fs::write(path.join("credentials.yaml"), "invalid: [").unwrap();
        let config = crate::config::load_from(&config_path).unwrap();
        let mut retained = None;

        assert!(
            credentials_for_startup(&config, &mut retained, &StartupBackend::Codex)
                .unwrap()
                .is_none()
        );
        assert!(retained.is_none());

        let native = StartupBackend::Native {
            provider: ProviderId::new("provider").unwrap(),
            account: AccountId::new("account").unwrap(),
            model: ModelId::new("model").unwrap(),
            replace_binding: false,
        };
        let error = credentials_for_startup(&config, &mut retained, &native).unwrap_err();

        fs::remove_dir_all(path).unwrap();
        assert!(error.to_string().contains("reading model credentials"));
        assert!(retained.is_none());
    }
}
