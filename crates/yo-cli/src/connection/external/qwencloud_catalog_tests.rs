use std::path::Path;

use yo_core::{
    AccountId, CompleteModelBinding, LocalConnectionRepository, ManagedConnectionAccount,
    ManagedConnectionBinding, ProviderId,
};

use super::*;
use crate::{
    AppError,
    command::ConnectCommand,
    connection::{
        input::{ExternalConnectInput, ModelPickerItem},
        presentation::Confirmation,
    },
};

struct CatalogCancelInput {
    selections: usize,
    credential_reads: usize,
}

impl ExternalConnectInput for CatalogCancelInput {
    fn confirm(&mut self, _: &Confirmation) -> Result<bool, AppError> {
        panic!("catalog cancellation must happen before confirmation")
    }

    fn read_credential(&mut self, _: &str) -> Result<yo_core::ApiCredential, AppError> {
        self.credential_reads += 1;
        panic!("catalog cancellation must happen before credential input")
    }

    fn select_model(&mut self, models: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        self.selections += 1;
        assert_eq!(models.len(), 18);
        let image = models
            .iter()
            .find(|model| model.model_id() == "qwen-image-2.0")
            .unwrap();
        assert!(!image.is_enabled());
        assert_eq!(image.disabled_reason(), Some("text output unsupported"));
        Ok(None)
    }
}

// QwenCloud picker는 operation recovery와 config capture 뒤 로컬 표만 보여 주며, 취소하면
// credential을 읽거나 plan·confirmation·세 repository mutation으로 진행하지 않습니다.
#[test]
fn qwencloud_catalog_cancellation_happens_before_secret_or_mutation() {
    let root = test_root("cancel");
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, token_plan_config()).unwrap();
    let mut input = CatalogCancelInput {
        selections: 0,
        credential_reads: 0,
    };

    let output = execute_external_connect_with_discovery(
        &config_path,
        command("qwencloud:team"),
        &mut input,
        |_, _, _| panic!("QwenCloud catalog must not call OpenRouter discovery"),
        |_, _, _, _, _| panic!("catalog cancellation must not finalize"),
    )
    .unwrap();

    assert_eq!(output, "Connection cancelled; nothing changed.\n");
    assert_eq!(input.selections, 1);
    assert_eq!(input.credential_reads, 0);
    assert_repositories_absent(&root);
    std::fs::remove_dir_all(root).unwrap();
}

struct CatalogSuccessInput {
    events: Vec<&'static str>,
}

impl ExternalConnectInput for CatalogSuccessInput {
    fn confirm(&mut self, _: &Confirmation) -> Result<bool, AppError> {
        self.events.push("confirm");
        Ok(true)
    }

    fn read_credential(&mut self, account: &str) -> Result<yo_core::ApiCredential, AppError> {
        assert_eq!(account, "qwencloud:team");
        self.events.push("credential");
        yo_core::ApiCredential::new("one-qwencloud-candidate")
            .map_err(|error| AppError::single("constructing catalog test credential", error))
    }

    fn select_model(&mut self, models: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        self.events.push("select");
        Ok(models
            .iter()
            .position(|model| model.model_id() == "qwen3.7-plus"))
    }
}

// local catalog에서 고른 exact 행 다음에 credential을 한 번 읽고 confirmation을 거쳐,
// 같은 complete binding과 candidate만 복구 가능한 publication 경계에 전달하는 순서를 검증합니다.
#[test]
fn qwencloud_catalog_selection_reuses_the_external_connect_transaction() {
    let root = test_root("success");
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, token_plan_config()).unwrap();
    let mut input = CatalogSuccessInput { events: Vec::new() };
    let mut finalized = false;

    let output = execute_external_connect_with_discovery(
        &config_path,
        command("qwencloud:team"),
        &mut input,
        |_, _, _| panic!("QwenCloud catalog must not call OpenRouter discovery"),
        |_, config, prepared, candidate, remote_selected| {
            finalized = true;
            assert!(!remote_selected);
            assert_eq!(candidate.expose_secret(), "one-qwencloud-candidate");
            let binding = &prepared.bindings()[0];
            assert_eq!(binding.binding().model_id().as_str(), "qwen3.7-plus");
            assert_eq!(
                binding.binding().endpoint().as_str(),
                "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
            );
            config.verify_unchanged().unwrap();
            Ok(())
        },
    )
    .unwrap();

    assert!(output.contains("qwencloud:team:qwen3.7-plus"));
    assert_eq!(input.events, ["select", "credential", "confirm"]);
    assert!(finalized);
    std::fs::remove_dir_all(root).unwrap();
}

struct ExactInput {
    selections: usize,
    credential_reads: usize,
}

impl ExternalConnectInput for ExactInput {
    fn confirm(&mut self, _: &Confirmation) -> Result<bool, AppError> {
        Ok(true)
    }

    fn read_credential(&mut self, _: &str) -> Result<yo_core::ApiCredential, AppError> {
        self.credential_reads += 1;
        yo_core::ApiCredential::new("exact-row-candidate")
            .map_err(|error| AppError::single("constructing exact-row credential", error))
    }

    fn select_model(&mut self, _: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        self.selections += 1;
        panic!("an exact QwenCloud row must bypass the picker")
    }
}

// exact Provider:Account:Model은 seed를 startup catalog로 확장하지 않아도 정확한 bundled
// 행을 찾아 picker 없이 기존 direct-target plan으로 들어가며 credential은 한 번만 읽습니다.
#[test]
fn exact_qwencloud_catalog_row_bypasses_the_picker() {
    let root = test_root("exact");
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, token_plan_config()).unwrap();
    let mut input = ExactInput {
        selections: 0,
        credential_reads: 0,
    };

    let output = execute_external_connect_with_discovery(
        &config_path,
        command("qwencloud:team:deepseek-v3.2"),
        &mut input,
        |_, _, _| panic!("an exact QwenCloud row must not discover remotely"),
        |_, _, prepared, _, _| {
            assert_eq!(
                prepared.bindings()[0].binding().model_id().as_str(),
                "deepseek-v3.2"
            );
            Ok(())
        },
    )
    .unwrap();

    assert!(output.contains("qwencloud:team:deepseek-v3.2"));
    assert_eq!(input.selections, 0);
    assert_eq!(input.credential_reads, 1);
    std::fs::remove_dir_all(root).unwrap();
}

// exact disabled 행과 catalog 밖 행은 credential이나 repository를 열기 전에 각각 안정적인
// interface 사유와 수동 binding 안내로 실패하고 fallback model을 추측하지 않습니다.
#[test]
fn exact_qwencloud_catalog_rejects_disabled_or_unknown_rows_before_secret() {
    for (target, expected) in [
        ("qwencloud:team:qwen-image-2.0", "text output unsupported"),
        (
            "qwencloud:team:not-in-the-plan",
            "use an explicit manual binding",
        ),
    ] {
        let root = test_root("reject");
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, token_plan_config()).unwrap();
        let mut input = ExactInput {
            selections: 0,
            credential_reads: 0,
        };
        let error = execute_external_connect_with_discovery(
            &config_path,
            command(target),
            &mut input,
            |_, _, _| panic!("exact catalog rows do not discover remotely"),
            |_, _, _, _, _| panic!("rejected rows do not finalize"),
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(input.credential_reads, 0);
        assert_repositories_absent(&root);
        std::fs::remove_dir_all(root).unwrap();
    }
}

// 이전 release에서 저장한 managed QwenCloud 행은 startup과 recovery용 durable 상태로 남지만,
// 현재 configured catalog 밖이면 새 connect 후보로 재사용되지 않는지 판별합니다.
#[test]
fn managed_qwencloud_row_outside_the_current_catalog_cannot_bypass_admission() {
    let root = test_root("removed-managed-row");
    let config_path = root.join("config.yaml");
    std::fs::write(&config_path, token_plan_config()).unwrap();
    let repository = LocalConnectionRepository::new(root.join("connections.yaml"));
    let account = ManagedConnectionAccount::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("team").unwrap(),
        Some("QwenCloud".to_owned()),
        Some("Token Plan Team".to_owned()),
    )
    .unwrap();
    let complete = CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"team","model":"removed-model","connector":"openai-chat-completions","base_url":"https://example.test/v1","api_dialect":"openai-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap();
    let binding =
        ManagedConnectionBinding::new(complete, Some("Removed Model".to_owned())).unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_managed_upsert(account, binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let before = std::fs::read(root.join("connections.yaml")).unwrap();
    let mut input = ExactInput {
        selections: 0,
        credential_reads: 0,
    };

    let error = execute_external_connect_with_discovery(
        &config_path,
        command("qwencloud:team:removed-model"),
        &mut input,
        |_, _, _| panic!("exact catalog rows do not discover remotely"),
        |_, _, _, _, _| panic!("a removed catalog row must not finalize"),
    )
    .unwrap_err();

    assert!(error.to_string().contains("outside the configured catalog"));
    assert_eq!(input.credential_reads, 0);
    assert_eq!(
        std::fs::read(root.join("connections.yaml")).unwrap(),
        before
    );
    assert!(!root.join("credentials.yaml").exists());
    assert!(!root.join("connection-operation.yaml").exists());
    std::fs::remove_dir_all(root).unwrap();
}

fn command(target: &str) -> ConnectCommand {
    ConnectCommand {
        target: target.to_owned(),
        verbose: false,
        credential_file: None,
        yes: false,
    }
}

fn token_plan_config() -> &'static str {
    r#"
model:
  bindings:
    - provider: qwencloud
      account: team
      account_display_name: Token Plan Team
      catalog: qwencloud-token-plan-team-intl/v1
"#
}

fn test_root(label: &str) -> std::path::PathBuf {
    let root = super::super::canonical_test_temp_dir().join(format!(
        "yo-qwencloud-catalog-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn assert_repositories_absent(root: &Path) {
    for name in [
        "connections.yaml",
        "credentials.yaml",
        "connection-operation.yaml",
    ] {
        assert!(!root.join(name).exists(), "{name} must remain absent");
    }
}
