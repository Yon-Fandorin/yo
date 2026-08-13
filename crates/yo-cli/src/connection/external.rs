use std::path::Path;

use yo_core::{
    CompleteModelBinding, ConnectionSnapshot, ManagedConnectionAccount, ManagedConnectionBinding,
    ModelCatalog, ModelCatalogEntry, ModelSelection, PreparedConnectionMutation, StartupPolicy,
    StartupSelectionSources, StartupTarget, resolve_startup_target, verify_external_connection,
};

use super::{
    display_target,
    input::{ExternalConnectInput, TtyExternalConnectInput},
    operation_repositories,
};
use crate::{AppError, command::ConnectCommand, config};

pub(super) fn run_external_connect(
    config_path: &Path,
    command: ConnectCommand,
) -> Result<String, AppError> {
    let mut input = TtyExternalConnectInput::new();
    execute_external_connect_with(config_path, command, &mut input)
}

fn execute_external_connect_with(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut impl ExternalConnectInput,
) -> Result<String, AppError> {
    let repositories = operation_repositories(config_path)?;
    let mut session = repositories
        .acquire()
        .map_err(|error| AppError::single("acquiring the connection operation lane", error))?;
    session
        .recover_pending_operation()
        .map_err(|error| AppError::single("recovering a pending connection operation", error))?;

    let config = config::load_from(config_path)
        .map_err(|error| AppError::single("reading Yo configuration", error))?;
    let snapshot = session
        .capture_connections()
        .map_err(|error| AppError::single("capturing managed connections", error))?;
    let selected = selected_entry(&snapshot, config.model_catalog(), &command.target)?;
    let selection = selection_for(&selected);
    let startup_policy = StartupPolicy::initial();
    let plan = ExternalConnectPlan::prepare(
        &snapshot,
        config.model_catalog(),
        &selection,
        &selected,
        &startup_policy,
    )?;
    let prepared = session
        .prepare_external_connection(
            config.snapshot_digest(),
            plan.connection,
            plan.verification_bindings,
        )
        .map_err(|error| AppError::single("preparing the external connection", error))?;

    if !input.confirm(&plan.summary)? {
        return Ok("connection cancelled; no state changed\n".to_owned());
    }
    let account_reference = format!("{}:{}", selection.provider(), selection.account());
    let candidate = input.read_credential(&account_reference)?;
    let verified = verify_external_connection(prepared, candidate).map_err(|error| {
        AppError::message(format!(
            "verifying the candidate API key failed: {error}; remove or edit the rejected binding before retrying"
        ))
    })?;
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    session
        .commit_verified_external_connection(verified)
        .map_err(|error| AppError::single("publishing the external connection", error))?;

    Ok(format!(
        "connected: {}; verified bindings: {}; default: {}\n",
        selection.canonical_reference(),
        plan.binding_count,
        display_target(plan.preference.as_ref())
    ))
}

struct ExternalConnectPlan {
    connection: PreparedConnectionMutation,
    verification_bindings: Vec<CompleteModelBinding>,
    binding_count: usize,
    preference: Option<StartupTarget>,
    summary: String,
}

impl ExternalConnectPlan {
    fn prepare(
        snapshot: &ConnectionSnapshot,
        manual: &ModelCatalog,
        selection: &ModelSelection,
        selected: &ModelCatalogEntry,
        startup_policy: &StartupPolicy,
    ) -> Result<Self, AppError> {
        let complete = selected.complete_binding().cloned().ok_or_else(|| {
            AppError::message(
                "external connect requires an explicit Provider/Account profile and model binding; migrate this legacy catalog entry to model.bindings",
            )
        })?;
        let mut verification_bindings = Vec::new();
        for entry in manual
            .entries()
            .iter()
            .filter(|entry| same_account(entry, selection))
        {
            let retained = entry.complete_binding().cloned().ok_or_else(|| {
                AppError::message(format!(
                    "Provider {} Account {} still has a legacy model entry; migrate every retained model to model.bindings before rotating this account key",
                    selection.provider(),
                    selection.account()
                ))
            })?;
            if !verification_bindings.contains(&retained) {
                verification_bindings.push(retained);
            }
        }
        for retained in snapshot.managed_bindings().iter().filter(|retained| {
            let binding = retained.complete().binding();
            binding.provider_id() == selection.provider()
                && binding.account_id() == selection.account()
        }) {
            let complete = retained.complete().clone();
            if !verification_bindings.contains(&complete) {
                verification_bindings.push(complete);
            }
        }
        if !verification_bindings.contains(&complete) {
            verification_bindings.push(complete.clone());
        }

        let account = ManagedConnectionAccount::new(
            selection.provider().clone(),
            selection.account().clone(),
            selected.provider_display_name().map(str::to_owned),
            selected.account_display_name().map(str::to_owned),
        )
        .map_err(|error| AppError::single("preparing the managed account", error))?;
        let binding = ManagedConnectionBinding::new(
            complete,
            selected.model_display_name().map(str::to_owned),
        )
        .map_err(|error| AppError::single("preparing the managed model binding", error))?;
        let prospective_catalog = snapshot
            .compose_catalog_after_managed_upsert(manual, account.clone(), binding.clone())
            .map_err(|error| {
                AppError::single("composing the prospective managed connection", error)
            })?;
        admit_external_target(&prospective_catalog, selection, startup_policy)?;
        let connection = snapshot
            .prepare_managed_connect(account, binding)
            .map_err(|error| AppError::single("preparing managed connection state", error))?;
        let preference = connection.preference().cloned();
        let binding_count = verification_bindings.len();
        let mut binding_summaries = verification_bindings
            .iter()
            .map(complete_binding_summary)
            .collect::<Vec<_>>();
        binding_summaries.sort();
        let summary = format!(
            "Connect Provider {} / Account {} and send one new API key to these {binding_count} exact binding profile(s):\n  - {}",
            selection.provider(),
            selection.account(),
            binding_summaries.join("\n  - ")
        );
        Ok(Self {
            connection,
            verification_bindings,
            binding_count,
            preference,
            summary,
        })
    }
}

fn admit_external_target(
    catalog: &ModelCatalog,
    selection: &ModelSelection,
    startup_policy: &StartupPolicy,
) -> Result<(), AppError> {
    let reference = selection.canonical_reference();
    let admitted = resolve_startup_target(
        catalog,
        startup_policy,
        StartupSelectionSources {
            invocation: Some(&reference),
            stored_preference: None,
            operator_target: None,
        },
    )
    .map_err(|error| AppError::single("admitting the external connection target", error))?;
    if admitted == Some(StartupTarget::Model(selection.clone())) {
        Ok(())
    } else {
        Err(AppError::message(
            "external connection policy did not admit the exact requested model target",
        ))
    }
}

fn complete_binding_summary(complete: &CompleteModelBinding) -> String {
    let binding = complete.binding();
    let profile = complete.profile();
    format!(
        "{} [endpoint={}, dialect={}, connector={}, tokenizer={}, input_limit={}, output_limit={}, reasoning={}, optional={}, tools={}, verification={}]",
        selection_for_binding(binding).canonical_reference(),
        binding.endpoint(),
        binding.api_dialect(),
        binding.connector_id(),
        profile.context().tokenizer_profile(),
        profile.context().input_token_limit(),
        profile.context().max_output_tokens(),
        profile.reasoning_parameters().to_json_value(),
        profile.optional_request_parameters().to_json_value(),
        profile.tool_capability_policy(),
        profile.verification_profile(),
    )
}

fn selected_entry(
    snapshot: &ConnectionSnapshot,
    manual: &ModelCatalog,
    reference: &str,
) -> Result<ModelCatalogEntry, AppError> {
    if let Some(selected) = manual
        .entries()
        .iter()
        .find(|entry| selection_for(entry).canonical_reference() == reference)
        .cloned()
    {
        return Ok(selected);
    }
    snapshot
        .compose_catalog(&ModelCatalog::default())
        .map_err(|error| AppError::single("reading the managed model catalog", error))?
        .entries()
        .iter()
        .find(|entry| selection_for(entry).canonical_reference() == reference)
        .cloned()
        .ok_or_else(|| {
            AppError::message(format!(
                "external connect target {reference:?} is not an exact configured Provider:Account:Model reference"
            ))
        })
}

fn same_account(entry: &ModelCatalogEntry, selection: &ModelSelection) -> bool {
    let binding = entry.binding();
    binding.provider_id() == selection.provider() && binding.account_id() == selection.account()
}

fn selection_for(entry: &ModelCatalogEntry) -> ModelSelection {
    selection_for_binding(entry.binding())
}

fn selection_for_binding(binding: &yo_core::EffectiveModelBinding) -> ModelSelection {
    ModelSelection::new(
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CancelInput {
        summary: Option<String>,
        credential_reads: usize,
    }

    impl ExternalConnectInput for CancelInput {
        fn confirm(&mut self, summary: &str) -> Result<bool, AppError> {
            self.summary = Some(summary.to_owned());
            Ok(false)
        }

        fn read_credential(&mut self, _: &str) -> Result<yo_core::ApiCredential, AppError> {
            self.credential_reads += 1;
            yo_core::ApiCredential::new("unreachable").map_err(|error| {
                AppError::single("constructing the unreachable test credential", error)
            })
        }
    }

    // 실제 external command 경로에서 exact target의 전체 확인문을 먼저 보여 주고 사용자가
    // 거절하면 credential을 읽거나 세 repository 파일을 만들지 않습니다.
    #[test]
    fn cancelled_command_stops_before_secret_or_repository_mutation() {
        let root = std::env::temp_dir().join(format!(
            "yo-external-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, explicit_config()).unwrap();
        let mut input = CancelInput {
            summary: None,
            credential_reads: 0,
        };

        let output = execute_external_connect_with(
            &config_path,
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
            },
            &mut input,
        )
        .unwrap();

        assert_eq!(output, "connection cancelled; no state changed\n");
        assert!(input.summary.unwrap().contains("vendor:team:alpha"));
        assert_eq!(input.credential_reads, 0);
        for name in [
            "connections.yaml",
            "credentials.yaml",
            "connection-operation.yaml",
        ] {
            assert!(!root.join(name).exists(), "{name} must remain absent");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // 같은 Account에 남는 모든 complete binding을 확인 목록에 포함해야 키 교체 전에
    // 하나라도 누락한 구현을 summary와 binding_count가 함께 구별합니다.
    #[test]
    fn plan_includes_every_retained_binding_for_the_account() {
        let catalog = fixture_catalog();
        let snapshot = fixture_snapshot();
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );

        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 2);
        assert!(plan.summary.contains("vendor:team:alpha"));
        assert!(plan.summary.contains("vendor:team:beta"));
        assert_eq!(plan.preference, Some(StartupTarget::Model(selection)));
    }

    // 같은 Account에 legacy binding이 하나라도 남으면 정확한 profile로 검증할 수 없으므로
    // credential/public mutation 실행 전 planning이 migration 안내로 실패합니다.
    #[test]
    fn plan_rejects_a_retained_legacy_binding() {
        let mut entries = fixture_catalog().entries().to_vec();
        let binding = yo_core::EffectiveModelBinding::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("legacy").unwrap(),
            yo_core::ApiDialect::OpenAiResponses,
            yo_core::NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        );
        entries.push(
            yo_core::ModelCatalogEntry::new(
                binding,
                Some("Vendor".to_owned()),
                Some("Team".to_owned()),
                None,
                yo_core::ModelContextProfile::new(1000, 100, "utf8-bytes/v1").unwrap(),
            )
            .unwrap(),
        );
        let catalog = ModelCatalog::new(entries).unwrap();
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );

        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let error = ExternalConnectPlan::prepare(
            &fixture_snapshot(),
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .err()
        .expect("legacy retained binding must fail planning");

        assert!(error.to_string().contains("migrate every retained model"));
    }

    // config의 새 complete profile과 기존 managed profile이 같은 coordinate에서 달라도
    // connect planning은 둘 다 검증하고 새 profile로 교체한 prospective catalog를 승인합니다.
    #[test]
    fn plan_verifies_old_and_new_profiles_during_managed_replacement() {
        let catalog = ModelCatalog::new(vec![fixture_entry("alpha")]).unwrap();
        let snapshot = fixture_snapshot_with_managed(fixture_complete_at(
            "alpha",
            "https://old.example.test/v1",
            "openai-chat-completions",
            r#"{"effort":"medium"}"#,
        ));
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();

        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 2);
        assert_eq!(plan.summary.matches("vendor:team:alpha").count(), 2);
        assert!(plan.summary.contains("endpoint=https://example.test/v1"));
        assert!(
            plan.summary
                .contains("endpoint=https://old.example.test/v1")
        );
        assert!(plan.summary.contains("dialect=openai-responses"));
        assert!(plan.summary.contains("dialect=openai-chat-completions"));
        assert!(plan.summary.contains("reasoning={}"));
        assert!(plan.summary.contains(r#"reasoning={"effort":"medium"}"#));
    }

    // external connect도 startup target policy를 통과해야 하므로 Host 강제 policy에서는
    // exact external target을 확인하거나 credential을 읽기 전에 planning이 거절됩니다.
    #[test]
    fn plan_enforces_target_policy_before_confirmation() {
        let catalog = fixture_catalog();
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let enforced_host =
            StartupPolicy::new(false, Some(StartupTarget::HostCodex), None).unwrap();

        let error = ExternalConnectPlan::prepare(
            &fixture_snapshot(),
            &catalog,
            &selection,
            selected,
            &enforced_host,
        )
        .err()
        .expect("the enforced Host policy must reject an external target");

        assert!(
            error
                .to_string()
                .contains("conflicts with the enforced startup policy")
        );
    }

    fn fixture_catalog() -> ModelCatalog {
        ModelCatalog::new(vec![fixture_entry("alpha"), fixture_entry("beta")]).unwrap()
    }

    fn explicit_config() -> &'static str {
        r#"version: 1
model:
  bindings:
    - provider: vendor
      account: team
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: {}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: alpha
"#
    }

    fn fixture_entry(model: &str) -> ModelCatalogEntry {
        let complete = fixture_complete(model);
        ModelCatalogEntry::with_explicit_profile(
            complete.binding().clone(),
            Some("Vendor".to_owned()),
            Some("Team".to_owned()),
            Some(model.to_owned()),
            complete.profile().clone(),
        )
        .unwrap()
    }

    fn fixture_complete(model: &str) -> CompleteModelBinding {
        fixture_complete_with_reasoning(model, "{}")
    }

    fn fixture_complete_with_reasoning(model: &str, reasoning: &str) -> CompleteModelBinding {
        fixture_complete_at(
            model,
            "https://example.test/v1",
            "openai-responses",
            reasoning,
        )
    }

    fn fixture_complete_at(
        model: &str,
        endpoint: &str,
        dialect: &str,
        reasoning: &str,
    ) -> CompleteModelBinding {
        CompleteModelBinding::from_durable_json(&format!(
            r#"{{"provider":"vendor","account":"team","model":"{model}","connector":"{dialect}","base_url":"{endpoint}","api_dialect":"{dialect}","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{reasoning},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}}"#
        ))
        .unwrap()
    }

    fn fixture_snapshot_with_managed(complete: CompleteModelBinding) -> ConnectionSnapshot {
        let root = std::env::temp_dir().join(format!(
            "yo-external-plan-managed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repository = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"));
        let account = ManagedConnectionAccount::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            Some("Vendor".to_owned()),
            Some("Team".to_owned()),
        )
        .unwrap();
        let binding = ManagedConnectionBinding::new(complete, Some("alpha".to_owned())).unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_connect(account, binding)
            .unwrap();
        repository.commit(&mutation).unwrap();
        let snapshot = repository.capture().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        snapshot
    }

    fn fixture_snapshot() -> ConnectionSnapshot {
        yo_core::LocalConnectionRepository::new(std::env::temp_dir().join(format!(
            "yo-external-plan-{}-missing/connections.yaml",
            std::process::id()
        )))
        .capture()
        .unwrap()
    }
}
