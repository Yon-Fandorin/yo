use std::path::Path;

use super::*;
use crate::{
    AppError,
    command::ConnectCommand,
    connection::{input::ExternalConnectInput, presentation::Confirmation},
};

// 두 부분 target은 정확히 configured OpenRouter/Kimi discovery와 QwenCloud catalog에만
// 예약하고, 다른 Provider나 세 부분 exact ModelTarget과 섞이지 않는지 판별합니다.
#[test]
fn recognizes_only_the_closed_two_part_onboarding_shapes() {
    let root = super::super::canonical_test_temp_dir().join(format!(
        "yo-catalog-pairs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    for definition in [
        discovery_definition(),
        "provider: qwencloud\naccount: team\ncatalog: qwencloud-coding-plan-intl/v1\n",
        "provider: kimi\naccount: team\ncatalog: kimi-platform-ai/v1\n",
    ] {
        seed_stored_definition(&root, definition);
    }
    let snapshot = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"))
        .capture()
        .unwrap();

    let (provider, account) = catalog_pair(&snapshot, "openrouter:team").unwrap().unwrap();
    assert_eq!(provider.as_str(), "openrouter");
    assert_eq!(account.as_str(), "team");
    let (provider, account) = catalog_pair(&snapshot, "qwencloud:team").unwrap().unwrap();
    assert_eq!(provider.as_str(), "qwencloud");
    assert_eq!(account.as_str(), "team");
    let (provider, account) = catalog_pair(&snapshot, "kimi:team").unwrap().unwrap();
    assert_eq!(provider.as_str(), "kimi");
    assert_eq!(account.as_str(), "team");
    assert!(
        catalog_pair(&snapshot, "openrouter:team:model")
            .unwrap()
            .is_none()
    );
    assert!(
        catalog_pair(&snapshot, "vendor:team")
            .unwrap_err()
            .to_string()
            .contains("unsupported")
    );
    std::fs::remove_dir_all(root).unwrap();
}

// discovery는 모델을 추측할 수 없는 interactive-only 흐름이므로 file/yes 조합도 config나
// credential 파일을 열기 전에 명시적으로 거절합니다.
#[test]
fn discovery_rejects_non_interactive_options_before_io() {
    for target in ["openrouter:team", "qwencloud:team", "kimi:team"] {
        for (credential_file, yes) in [
            (Some(std::path::PathBuf::from("/not/read/credential")), true),
            (
                Some(std::path::PathBuf::from("/not/read/credential")),
                false,
            ),
            (None, true),
        ] {
            let error = run_external_connect(
                Path::new("/not/read/config.yaml"),
                ConnectCommand {
                    from: None,
                    target: target.to_owned(),
                    verbose: false,
                    credential_file,
                    yes,
                },
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("interactive only"));
        }
    }
}

struct DiscoveryCancelInput {
    credential_reads: usize,
}

impl ExternalConnectInput for DiscoveryCancelInput {
    fn confirm(&mut self, _: &Confirmation) -> Result<bool, AppError> {
        panic!("discovery cancellation must happen before confirmation")
    }

    fn read_credential(&mut self, account: &str) -> Result<yo_core::ApiCredential, AppError> {
        assert_eq!(account, "openrouter:team");
        self.credential_reads += 1;
        yo_core::ApiCredential::new("one-candidate-secret")
            .map_err(|error| AppError::single("constructing discovery test credential", error))
    }
}

// 실제 command orchestration이 lock/recovery/config seed 뒤 candidate를 정확히 한 번 읽고
// discovery picker 취소 시 plan·preview·세 repository mutation 전에 멈추는지 판별합니다.
#[test]
fn discovery_cancellation_discards_the_candidate_before_mutation() {
    let root = super::super::canonical_test_temp_dir().join(format!(
        "yo-external-discovery-cancel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, "session: {}\n").unwrap();
    seed_stored_definition(&root, discovery_definition());
    let before = std::fs::read(root.join("connections.yaml")).unwrap();
    let mut input = DiscoveryCancelInput {
        credential_reads: 0,
    };
    let mut discovery_calls = 0;

    let output = execute_external_connect_with_discovery(
        &config_path,
        ConnectCommand {
            from: None,
            target: "openrouter:team".to_owned(),
            verbose: false,
            credential_file: None,
            yes: false,
        },
        &mut input,
        |seed, candidate, _| {
            discovery_calls += 1;
            assert_eq!(seed.provider().as_str(), "openrouter");
            assert_eq!(seed.account().as_str(), "team");
            assert_eq!(candidate.expose_secret(), "one-candidate-secret");
            Ok(None)
        },
        |_, _, _, _, _| panic!("cancellation must not reach final publication"),
    )
    .unwrap();

    assert_eq!(output, "Connection cancelled; nothing changed.\n");
    assert_eq!(input.credential_reads, 1);
    assert_eq!(discovery_calls, 1);
    assert_eq!(
        std::fs::read(root.join("connections.yaml")).unwrap(),
        before
    );
    for name in ["credentials.yaml", "connection-operation.yaml"] {
        assert!(!root.join(name).exists(), "{name} must remain absent");
    }
    std::fs::remove_dir_all(root).unwrap();
}

struct DiscoverySuccessInput {
    credential_reads: usize,
    confirmations: usize,
}

impl ExternalConnectInput for DiscoverySuccessInput {
    fn confirm(&mut self, _: &Confirmation) -> Result<bool, AppError> {
        self.confirmations += 1;
        Ok(true)
    }

    fn read_credential(&mut self, account: &str) -> Result<yo_core::ApiCredential, AppError> {
        assert_eq!(account, "openrouter:team");
        self.credential_reads += 1;
        yo_core::ApiCredential::new("one-candidate-secret")
            .map_err(|error| AppError::single("constructing discovery test credential", error))
    }
}

// discovery가 반환한 exact row와 그 catalog를 읽은 candidate 하나가 그대로 final
// publication boundary에 도달해, 선택 뒤 credential 재입력이나 다른 model 선택을 막습니다.
#[test]
fn successful_discovery_binds_one_candidate_and_selected_row_to_publication() {
    let root = super::super::canonical_test_temp_dir().join(format!(
        "yo-external-discovery-success-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, "session: {}\n").unwrap();
    seed_stored_definition(&root, discovery_definition());
    let mut input = DiscoverySuccessInput {
        credential_reads: 0,
        confirmations: 0,
    };
    let selected = discovered_entry("vendor/selected");
    let selected_for_discovery = selected.clone();
    let mut finalizations = 0;

    let output = execute_external_connect_with_discovery(
        &config_path,
        ConnectCommand {
            from: None,
            target: "openrouter:team".to_owned(),
            verbose: false,
            credential_file: None,
            yes: false,
        },
        &mut input,
        |seed, candidate, _| {
            assert_eq!(seed.provider().as_str(), "openrouter");
            assert_eq!(seed.account().as_str(), "team");
            assert_eq!(candidate.expose_secret(), "one-candidate-secret");
            Ok(Some(selected_for_discovery))
        },
        |_, config, prepared, candidate, discovered| {
            finalizations += 1;
            assert!(discovered);
            assert_eq!(candidate.expose_secret(), "one-candidate-secret");
            assert_eq!(
                prepared.bindings(),
                std::slice::from_ref(selected.complete_binding().unwrap())
            );
            config.verify_unchanged().unwrap();
            Ok(())
        },
    )
    .unwrap();

    assert!(output.contains("openrouter:team:vendor/selected"));
    assert_eq!(input.credential_reads, 1);
    assert_eq!(input.confirmations, 1);
    assert_eq!(finalizations, 1);
    std::fs::remove_dir_all(root).unwrap();
}

fn discovered_entry(model: &str) -> yo_core::ModelCatalogEntry {
    let complete = yo_core::CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"openrouter","account":"team","model":"{model}","connector":"openai-responses","base_url":"https://openrouter.ai/api/v1","api_dialect":"openai-responses","tokenizer_profile":"o200k_base/v1","input_token_limit":120000,"max_output_tokens":12000,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    ))
    .unwrap();
    yo_core::ModelCatalogEntry::with_explicit_profile(
        complete.binding().clone(),
        Some("OpenRouter".to_owned()),
        Some("Team".to_owned()),
        Some("Selected".to_owned()),
        complete.profile().clone(),
    )
    .unwrap()
}

fn discovery_definition() -> &'static str {
    r#"
provider: openrouter
provider_display_name: OpenRouter
account: team
account_display_name: Team
base_url: https://openrouter.ai/api/v1
profile:
  api_dialect: openai-responses
  tokenizer_profile: o200k_base/v1
  input_token_limit: 200000
  max_output_tokens: 16000
  reasoning_parameters: {}
  optional_request_parameters: {}
  tool_capability_policy: local-tools/v1
"#
}
