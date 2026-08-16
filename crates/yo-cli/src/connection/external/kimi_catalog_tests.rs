use super::*;
use crate::{
    AppError,
    command::ConnectCommand,
    connection::{input::ExternalConnectInput, presentation::Confirmation},
};

struct KimiInput {
    events: Vec<&'static str>,
    preview: Option<String>,
}

impl ExternalConnectInput for KimiInput {
    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError> {
        self.events.push("confirm");
        self.preview = Some(
            preview
                .render(super::super::presentation::default_width())
                .unwrap(),
        );
        Ok(true)
    }

    fn read_credential(&mut self, account: &str) -> Result<yo_core::ApiCredential, AppError> {
        assert_eq!(account, "kimi:team");
        self.events.push("credential");
        yo_core::ApiCredential::new("one-kimi-candidate")
            .map_err(|error| AppError::single("constructing Kimi test credential", error))
    }
}

// 인증 inventory에서 선택된 Kimi K3 complete binding이 candidate 하나와 함께 기존
// transaction으로 전달되고, commit 승인 전에 plaintext private replay 경고가 보입니다.
#[test]
fn kimi_discovery_selection_reaches_preview_and_final_verification_unchanged() {
    let root = test_root("success");
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, kimi_config()).unwrap();
    let selected = kimi_k3_entry();
    let selected_for_discovery = selected.clone();
    let mut input = KimiInput {
        events: Vec::new(),
        preview: None,
    };
    let mut finalized = false;

    let output = execute_external_connect_with_catalogs(
        &config_path,
        command(),
        &mut input,
        |_, _, _| panic!("Kimi target must not call OpenRouter discovery"),
        |seed, candidate, _| {
            assert_eq!(seed.provider().as_str(), "kimi");
            assert_eq!(seed.account().as_str(), "team");
            assert_eq!(candidate.expose_secret(), "one-kimi-candidate");
            Ok(Some(selected_for_discovery))
        },
        |_, config, prepared, candidate, remote_selected| {
            finalized = true;
            assert!(remote_selected);
            assert_eq!(candidate.expose_secret(), "one-kimi-candidate");
            assert_eq!(
                prepared.verification_bindings(),
                std::slice::from_ref(selected.complete_binding().unwrap())
            );
            let profile = prepared.verification_bindings()[0].profile();
            assert_eq!(
                profile.replay_profile().as_str(),
                yo_core::KIMI_PRIVATE_REPLAY_PROFILE
            );
            config.verify_unchanged().unwrap();
            Ok(())
        },
    )
    .unwrap();

    assert!(output.contains("kimi:team:kimi-k3"));
    assert_eq!(input.events, ["credential", "confirm"]);
    let preview = input.preview.unwrap();
    assert!(preview.contains("Private replay"));
    assert!(preview.contains("unencrypted"));
    assert!(finalized);
    std::fs::remove_dir_all(root).unwrap();
}

fn command() -> ConnectCommand {
    ConnectCommand {
        target: "kimi:team".to_owned(),
        verbose: false,
        credential_file: None,
        yes: false,
    }
}

fn kimi_k3_entry() -> ModelCatalogEntry {
    let complete = CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"team","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap();
    ModelCatalogEntry::with_explicit_profile(
        complete.binding().clone(),
        Some("Kimi".to_owned()),
        Some("Team".to_owned()),
        Some("Kimi K3".to_owned()),
        complete.profile().clone(),
    )
    .unwrap()
}

fn kimi_config() -> &'static str {
    r#"version: 1
model:
  bindings:
    - provider: kimi
      provider_display_name: Kimi
      account: team
      account_display_name: Team
      catalog: kimi-platform-ai/v1
"#
}

fn test_root(label: &str) -> std::path::PathBuf {
    let root = super::super::canonical_test_temp_dir().join(format!(
        "yo-kimi-catalog-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}
