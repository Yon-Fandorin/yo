use std::{fs, path::PathBuf, time::SystemTime};

use yo_core::{
    AccountId, ApiCredential, CompleteModelBinding, LocalConnectionRepository,
    LocalCredentialRepository, ManagedConnectionAccount, ModelId, ProviderId,
};

use super::*;

struct FakeInput {
    selected: Option<String>,
    confirmed: bool,
    selections: Vec<Vec<String>>,
    summaries: Vec<String>,
}

impl ExternalDisconnectInput for FakeInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError> {
        self.selections.push(choices.to_vec());
        self.selected
            .clone()
            .ok_or_else(|| AppError::message("no fake selection"))
    }

    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError> {
        self.summaries.push(
            preview
                .render(super::super::presentation::default_width())
                .unwrap(),
        );
        Ok(self.confirmed)
    }
}

// --yes는 exact Provider/Account 아래 managed target이 하나일 때만 TTY 없이 실행하고,
// public binding과 matching preference를 지운 뒤 마지막 dependent credential도 제거합니다.
#[test]
fn automatic_unique_disconnect_removes_public_then_credential_without_prompt() {
    let fixture = Fixture::new("automatic");
    let config_path = fixture.config_path("version: 1\n");
    fixture.seed_managed(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(
        output,
        "✓ Disconnected\n\n  Model    vendor:team:alpha\n  API key  Removed\n  Default  unset\n"
    );
    assert!(input.selections.is_empty());
    assert!(input.summaries.is_empty());
    assert!(
        fixture
            .connections()
            .capture()
            .unwrap()
            .managed_bindings()
            .is_empty()
    );
    assert!(
        fixture
            .credentials()
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_none()
    );
    assert!(!fixture.root.join("connection-operation.yaml").exists());
}

// 여러 managed target 중 하나를 대화형으로 고르면 preview가 exact removed profile,
// preference 전이, 남는 distinct binding, credential preserve와 resume risk를 모두 표시합니다.
#[test]
fn interactive_preview_selects_one_and_discloses_the_complete_preserve_plan() {
    let fixture = Fixture::new("interactive");
    let config_path = fixture.config_path("version: 1\n");
    fixture.seed_managed(&["alpha", "beta"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: Some("vendor:team:beta".to_owned()),
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: true,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(output, "Disconnect cancelled; nothing changed.\n");
    assert_eq!(
        input.selections,
        [vec![
            "vendor:team:alpha".to_owned(),
            "vendor:team:beta".to_owned()
        ]]
    );
    let summary = &input.summaries[0];
    assert!(summary.contains("Connection being removed"));
    assert!(summary.contains("vendor:team:beta"));
    assert!(summary.contains("Keep vendor:team:alpha"));
    assert!(summary.contains("Keep — still used by vendor:team:alpha"));
    assert!(summary.contains("vendor:team:alpha"));
    assert!(summary.contains("Unavailable until this exact model is restored"));
    assert_eq!(
        fixture
            .connections()
            .capture()
            .unwrap()
            .managed_bindings()
            .len(),
        2
    );
    assert!(
        fixture
            .credentials()
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_some()
    );

    let mut compact_input = FakeInput {
        selected: Some("vendor:team:beta".to_owned()),
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };
    execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        },
        &mut compact_input,
    )
    .unwrap();
    let compact = &compact_input.summaries[0];
    assert!(compact.contains("Keep — still used by vendor:team:alpha"));
    assert!(!compact.contains("Connection being removed"));
    assert!(!compact.contains("Still available for this account"));
}

// 같은 complete binding의 manual provenance가 남으면 managed provenance만 제거하고
// post-public catalog의 manual dependency 때문에 credential은 preserve로 계획됩니다.
#[test]
fn equal_manual_binding_preserves_credential_and_preview_names_provenance_transition() {
    let fixture = Fixture::new("manual-equal");
    let config_path = fixture.config_path(&explicit_config("alpha"));
    fixture.seed_managed(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: false,
            verbose: true,
        },
        &mut input,
    )
    .unwrap();

    assert!(output.contains("API key  Kept"));
    assert!(
        input.summaries[0].contains("Managed copy removed; equal manual configuration remains")
    );
    assert!(input.summaries[0].contains("Resume through equal manual configuration"));
    assert!(
        fixture
            .credentials()
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_some()
    );
}

// 저장 preference를 제거해도 더 낮은 startup source가 있으면 preview는 막연한 재설정
// 경고 대신 실제 prospective resolver가 선택할 exact target을 보여 줍니다.
#[test]
fn preview_resolves_the_exact_lower_priority_startup_target() {
    let fixture = Fixture::new("startup-fallback");
    fixture.seed_managed(&["alpha"], Some("alpha"));
    let snapshot = fixture.connections().capture().unwrap();
    let selection = ModelSelection::new(provider(), account(), ModelId::new("alpha").unwrap());
    let policies = [
        (
            StartupPolicy::new(true, None, Some(StartupTarget::HostCodex)).unwrap(),
            None,
        ),
        (
            StartupPolicy::new(false, Some(StartupTarget::HostCodex), None).unwrap(),
            None,
        ),
        (StartupPolicy::initial(), Some(StartupTarget::HostCodex)),
    ];

    for (policy, operator_target) in policies {
        let plan = ExternalDisconnectPlan::prepare(
            &snapshot,
            &ModelCatalog::default(),
            &selection,
            &policy,
            operator_target,
            false,
        )
        .unwrap();
        let preview = plan
            .preview
            .render(super::super::presentation::default_width())
            .unwrap();

        assert!(preview.contains("✓ New sessions\n  Use host:codex"));
        assert!(!preview.contains("No startup target remains"));
    }
}

// 실제 disconnect command는 command-local config.yaml의 operator model.startup을 capture해
// preference 제거 뒤 새 Session이 사용할 exact fallback으로 preview에 전달합니다.
#[test]
fn command_preview_uses_the_captured_operator_startup_target() {
    let fixture = Fixture::new("operator-startup");
    let config_path = fixture.config_path("version: 1\nmodel:\n  startup: host:codex\n");
    fixture.seed_managed(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(output, "Disconnect cancelled; nothing changed.\n");
    assert!(input.summaries[0].contains("✓ New sessions\n  Use host:codex"));
    assert!(!input.summaries[0].contains("No startup target remains"));
    assert_eq!(
        fixture.connections().capture().unwrap().preference(),
        Some(&StartupTarget::Model(ModelSelection::new(
            provider(),
            account(),
            ModelId::new("alpha").unwrap(),
        )))
    );
}

// --yes 범위가 둘 이상의 target이면 모델을 추측하지 않고 실패하며, manual-only 범위도
// managed state를 만들지 않고 config.yaml을 편집하라는 정확한 소유권 안내를 반환합니다.
#[test]
fn automatic_ambiguity_and_manual_only_targets_fail_before_mutation() {
    let fixture = Fixture::new("selection-errors");
    let config_path = fixture.config_path(&explicit_config("manual"));
    fixture.seed_managed(&["alpha", "beta"], None);
    let mut input = FakeInput {
        selected: None,
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };
    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();
    assert!(error.to_string().contains("--yes never guesses"));

    let manual_only = Fixture::new("manual-only");
    let manual_config = manual_only.config_path(&explicit_config("manual"));
    let error = execute_external_disconnect_with(
        &manual_config,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();
    assert!(error.to_string().contains("edit config.yaml"));
    assert!(!manual_only.root.join("connections.yaml").exists());
}

// 마지막 managed binding을 대화형으로 제거하는 preview는 credential remove뿐 아니라
// 해당 complete binding에 귀속된 기존 Session이 native resume되지 않을 위험도 명시합니다.
#[test]
fn last_binding_preview_warns_about_remove_continuation_risk() {
    let fixture = Fixture::new("remove-resume-risk");
    let config_path = fixture.config_path("version: 1\n");
    fixture.seed_managed(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(output, "Disconnect cancelled; nothing changed.\n");
    assert!(input.summaries[0].contains("Remove — no configured model uses vendor:team"));
    assert!(input.summaries[0].contains("Unavailable until this exact model is restored"));
    assert!(!input.summaries[0].contains("Connection being removed"));
    assert!(!input.summaries[0].contains("Still available for this account"));
}

struct ConfigChangingInput {
    config_path: PathBuf,
    confirmation_reads: usize,
}

impl ExternalDisconnectInput for ConfigChangingInput {
    fn select_target(&mut self, _: &[String]) -> Result<String, AppError> {
        Err(AppError::message("the unique target must not prompt"))
    }

    fn confirm(&mut self, _: &Confirmation) -> Result<bool, AppError> {
        self.confirmation_reads += 1;
        fs::write(&self.config_path, "version: 1\ntui:\n  max_fps: 60\n").unwrap();
        Ok(true)
    }
}

// 사람이 preview를 확인하는 동안 config.yaml이 바뀌면 exact command snapshot guard가
// journal/public/credential 쓰기보다 먼저 실패하고 세 저장소를 원래 상태로 유지합니다.
#[test]
fn changed_config_after_confirmation_aborts_before_disconnect_intent() {
    let fixture = Fixture::new("config-change");
    let config_path = fixture.config_path("version: 1\n");
    fixture.seed_managed(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let before_public = fs::read(fixture.connections().path()).unwrap();
    let before_credential = fs::read(fixture.credentials().path()).unwrap();
    let mut input = ConfigChangingInput {
        config_path: config_path.clone(),
        confirmation_reads: 0,
    };

    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("changed while this command was preparing")
    );
    assert_eq!(input.confirmation_reads, 1);
    assert_eq!(
        fs::read(fixture.connections().path()).unwrap(),
        before_public
    );
    assert_eq!(
        fs::read(fixture.credentials().path()).unwrap(),
        before_credential
    );
    assert!(!fixture.root.join("connection-operation.yaml").exists());
}

// 이전 operation journal이 손상돼 있으면 새 config/selection/TTY보다 recovery가 먼저
// 실패하여, 사용자가 새 disconnect를 승인해 기존 복구 문제를 덮는 일이 없습니다.
#[test]
fn pending_recovery_failure_precedes_new_disconnect_input() {
    let fixture = Fixture::new("recovery-first");
    let config_path = fixture.config_path("not valid: [");
    fs::write(fixture.root.join("connection-operation.yaml"), "pending\n").unwrap();
    let mut input = FakeInput {
        selected: None,
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();

    assert!(error.to_string().contains("pending connection operation"));
    assert!(!error.to_string().contains("invalid configuration"));
    assert!(input.selections.is_empty());
    assert!(input.summaries.is_empty());
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-cli-disconnect-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn config_path(&self, contents: &str) -> PathBuf {
        let path = self.root.join("config.yaml");
        fs::write(&path, contents).unwrap();
        path
    }

    fn connections(&self) -> LocalConnectionRepository {
        LocalConnectionRepository::new(self.root.join("connections.yaml"))
    }

    fn credentials(&self) -> LocalCredentialRepository {
        LocalCredentialRepository::new(self.root.join("credentials.yaml"))
    }

    fn seed_managed(&self, models: &[&str], preference: Option<&str>) {
        let repository = self.connections();
        for model in models {
            let account = ManagedConnectionAccount::new(
                provider(),
                account(),
                Some("Vendor".to_owned()),
                Some("Team".to_owned()),
            )
            .unwrap();
            let binding =
                ManagedConnectionBinding::new(complete(model), Some((*model).to_owned())).unwrap();
            let mutation = repository
                .capture()
                .unwrap()
                .prepare_managed_upsert(account, binding)
                .unwrap()
                .unwrap();
            repository.commit(&mutation).unwrap();
        }
        if let Some(model) = preference {
            let target = StartupTarget::Model(ModelSelection::new(
                provider(),
                account(),
                yo_core::ModelId::new(model).unwrap(),
            ));
            let mutation = repository
                .capture()
                .unwrap()
                .prepare_preference(Some(target))
                .unwrap();
            if let Some(mutation) = mutation {
                repository.commit(&mutation).unwrap();
            }
        }
    }

    fn seed_credential(&self) {
        let repository = self.credentials();
        let mutation = repository.prepare_set(&provider(), &account()).unwrap();
        repository
            .commit(
                &mutation,
                Some(&ApiCredential::new("fixture-secret").unwrap()),
            )
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn provider() -> ProviderId {
    ProviderId::new("vendor").unwrap()
}

fn account() -> AccountId {
    AccountId::new("team").unwrap()
}

fn complete(model: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"vendor","account":"team","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}}"#
    ))
    .unwrap()
}

fn explicit_config(model: &str) -> String {
    format!(
        "version: 1\nmodel:\n  bindings:\n    - provider: vendor\n      account: team\n      base_url: https://example.test/v1\n      profile:\n        api_dialect: openai-responses\n        tokenizer_profile: utf8-bytes/v1\n        input_token_limit: 1000\n        max_output_tokens: 100\n        reasoning_parameters: {{}}\n        optional_request_parameters: {{}}\n        tool_capability_policy: local-tools/v1\n        verification_profile: semantic-terminal/v1\n      models:\n        - model: {model}\n"
    )
}
