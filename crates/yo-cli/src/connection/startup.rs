use yo_core::StartupTarget;

use crate::{AppError, config::Config};

pub(crate) fn load_startup_connections(
    config: &mut Config,
) -> Result<Option<StartupTarget>, AppError> {
    let snapshot = super::operation::repository(config)
        .capture()
        .map_err(|error| AppError::single("reading stored connections", error))?;
    let preference = snapshot.preference().cloned();
    let catalog = snapshot
        .model_catalog()
        .map_err(|error| AppError::single("building the stored model catalog", error))?;
    config.replace_model_catalog(catalog);
    Ok(preference)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = super::super::operation::canonical_test_temp_dir()
                .join(format!("yo-cli-startup-{nonce}-{}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn config_path(&self) -> PathBuf {
            let path = self.0.join("config.yaml");
            fs::write(&path, "session: {}\n").unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // live startup은 preference와 typed stored catalog을 같은 repository snapshot에서 읽어
    // config에 없는 durable model target도 즉시 resolve할 수 있어야 합니다.
    #[test]
    fn startup_load_uses_stored_catalog_and_preference_from_one_snapshot() {
        let directory = TestDirectory::new();
        let config_path = directory.config_path();
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let durable = r#"{"provider":"qwencloud","account":"default","model":"stored","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#;
        let complete = yo_core::CompleteModelBinding::from_durable_json(durable).unwrap();
        let account = yo_core::ConnectionAccount::new(
            yo_core::ProviderId::new("qwencloud").unwrap(),
            yo_core::AccountId::new("default").unwrap(),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
        )
        .unwrap();
        let binding =
            yo_core::StoredModelBinding::new(complete, Some("Model stored".to_owned())).unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();

        let mut config = crate::config::load_from(&config_path).unwrap();
        let preference = load_startup_connections(&mut config).unwrap();

        assert!(matches!(preference, Some(StartupTarget::Model(_))));
        let entry = config
            .model_catalog()
            .resolve_model(
                &yo_core::ProviderId::new("qwencloud").unwrap(),
                &yo_core::AccountId::new("default").unwrap(),
                &yo_core::ModelId::new("stored").unwrap(),
            )
            .unwrap();
        assert_eq!(entry.model_display_name(), Some("Model stored"));
    }
}
