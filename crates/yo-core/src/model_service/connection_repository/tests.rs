use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::{AccountId, ModelId, ModelSelection, ProviderId, VersionedProfileId};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-connections-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository(name: &str) -> (TestDirectory, LocalConnectionRepository) {
    let directory = TestDirectory::new(name);
    let repository = LocalConnectionRepository::new(directory.0.join("nested/connections.yaml"));
    (directory, repository)
}

fn model_target(model: &str) -> StartupTarget {
    StartupTarget::Model(ModelSelection::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new(model).unwrap(),
    ))
}

fn stored_account() -> ConnectionAccount {
    ConnectionAccount::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        Some("QwenCloud".to_owned()),
        Some("Default".to_owned()),
    )
    .unwrap()
}

fn stored_binding(model: &str, effort: &str) -> StoredModelBinding {
    let durable = format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    );
    StoredModelBinding::new(
        crate::CompleteModelBinding::from_durable_json(&durable).unwrap(),
        Some(format!("Model {model}")),
    )
    .unwrap()
}

fn stored_binding_with_unknown_output(model: &str) -> StoredModelBinding {
    let durable = format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    );
    StoredModelBinding::new(
        crate::CompleteModelBinding::from_durable_json(&durable).unwrap(),
        Some(format!("Model {model}")),
    )
    .unwrap()
}

fn stored_kimi_binding() -> StoredModelBinding {
    StoredModelBinding::new(
        crate::CompleteModelBinding::from_durable_json(
            r#"{"provider":"kimi","account":"team","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
        )
        .unwrap(),
        Some("Kimi K3".to_owned()),
    )
    .unwrap()
}

fn stored_kimi_account() -> ConnectionAccount {
    ConnectionAccount::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        Some("Kimi".to_owned()),
        Some("Team".to_owned()),
    )
    .unwrap()
}

mod activation;
mod durable_profile;
mod group_mutation;
mod observation;
mod storage_cas;
