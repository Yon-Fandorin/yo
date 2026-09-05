use yo_core::{
    AccountId, ApiDialect, EffectiveModelBinding, HostCatalogModel, HostModelCatalog, ModelCatalog,
    ModelCatalogEntry, ModelContextProfile, ModelId, ModelSelection, ModelSelectionController,
    NormalizedEndpoint, ProviderId, derive_host_account_id, derive_host_catalog_revision,
};

use super::*;

// managed Session에서도 inventory 계획은 Codex와 Grok을 모두 포함해, 현재 backend가
// delegated host가 아니라는 이유로 host account section 전체가 사라지지 않게 합니다.
#[test]
fn managed_session_requests_both_builtin_host_inventories() {
    let requests = inventory_requests(None);

    assert_eq!(requests[0].host, HostId::codex());
    assert_eq!(requests[1].host, HostId::grok());
    assert_eq!(requests[0].execution, DelegatedExecutionProfile::Standard);
    assert_eq!(requests[1].execution, DelegatedExecutionProfile::Standard);
}

// 제한 실행 프로필은 정확한 활성 Grok에만 적용하고 sibling Codex inventory는 일반
// session-free profile로 남겨 다른 host의 정책을 전이하지 않습니다. Grok outer sandbox
// 정책은 provider owner인 host_catalog::grok 테스트가 검증합니다.
#[test]
fn active_read_only_grok_does_not_change_the_codex_inventory_profile() {
    let grok = HostId::grok();
    let requests = inventory_requests(Some((&grok, DelegatedExecutionProfile::ReadOnlyReview)));

    assert_eq!(requests[0].execution, DelegatedExecutionProfile::Standard);
    assert_eq!(
        requests[1].execution,
        DelegatedExecutionProfile::ReadOnlyReview
    );
}

// managed binding이 현재인 picker에도 두 host account와 exact advertised model을 모두
// 투영하되, 비활성 host가 보고한 자체 default에는 `(current)`를 붙이지 않습니다.
#[test]
fn managed_picker_projects_both_inactive_host_catalogs() {
    let managed = managed_selection();
    let controller = ModelSelectionController::new(
        ModelCatalog::new(vec![managed_entry()]).unwrap(),
        Some(managed),
    );
    let observations = vec![
        HostCatalogObservation::new(HostId::codex(), Ok(host_catalog(HostId::codex()))),
        HostCatalogObservation::new(HostId::grok(), Ok(host_catalog(HostId::grok()))),
    ];

    let controller = project_host_catalogs(controller, None, &observations);
    let sections = controller.sections();

    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].label(), "Qwen Cloud · Default");
    assert!(sections[0].choices()[0].is_current());
    assert!(
        sections
            .iter()
            .any(|section| section.label() == "Codex · codex@example.test")
    );
    assert!(
        sections
            .iter()
            .any(|section| section.label() == "Grok · grok@example.test")
    );
    assert!(
        sections[1..]
            .iter()
            .flat_map(yo_core::ModelPickerSection::choices)
            .all(|choice| !choice.is_current())
    );
    assert!(
        sections[1..]
            .iter()
            .flat_map(yo_core::ModelPickerSection::choices)
            .all(|choice| !choice.is_enabled()
                && choice.disabled_reason() == Some(SEMANTIC_HANDOFF_UNAVAILABLE))
    );
}

// delegated Grok이 활성인 picker는 Grok exact model만 current로 올리고, managed와 Codex
// section을 sibling으로 유지해 현재 host 하나가 unified catalog를 독점하지 않게 합니다.
#[test]
fn active_host_is_current_while_every_sibling_section_remains_visible() {
    let controller = ModelSelectionController::new(
        ModelCatalog::new(vec![managed_entry()]).unwrap(),
        Some(managed_selection()),
    );
    let grok_catalog = host_catalog(HostId::grok());
    let active = ActiveHostModel::new(
        HostId::grok(),
        grok_catalog.account().clone(),
        grok_catalog.current_model().unwrap().clone(),
        false,
    );
    let observations = vec![
        HostCatalogObservation::new(HostId::codex(), Ok(host_catalog(HostId::codex()))),
        HostCatalogObservation::new(HostId::grok(), Ok(grok_catalog)),
    ];

    let controller = project_host_catalogs(controller, Some(&active), &observations);
    let sections = controller.sections();

    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].label(), "Grok · grok@example.test");
    assert!(sections[0].choices()[0].label().ends_with(" (current)"));
    assert!(!sections[0].choices()[0].is_enabled());
    assert_eq!(
        sections[0].choices()[0].disabled_reason(),
        Some(NATIVE_REBIND_UNAVAILABLE)
    );
    assert!(
        sections
            .iter()
            .any(|section| section.label() == "Codex · codex@example.test")
    );
    assert!(
        sections
            .iter()
            .any(|section| section.label() == "Qwen Cloud · Default")
    );
}

// Codex가 native rebind를 광고한 같은 account에서만 exact model 행을 선택할 수 있고,
// Grok·managed sibling은 현재 구현되지 않은 handoff 경계로 남습니다.
#[test]
fn active_codex_enables_only_same_account_native_rebind_rows() {
    let codex_catalog = host_catalog(HostId::codex());
    let active = ActiveHostModel::new(
        HostId::codex(),
        codex_catalog.account().clone(),
        codex_catalog.current_model().unwrap().clone(),
        true,
    );
    let observations = vec![
        HostCatalogObservation::new(HostId::codex(), Ok(codex_catalog)),
        HostCatalogObservation::new(HostId::grok(), Ok(host_catalog(HostId::grok()))),
    ];

    let controller = project_host_catalogs(
        ModelSelectionController::new(
            ModelCatalog::new(vec![managed_entry()]).unwrap(),
            Some(managed_selection()),
        ),
        Some(&active),
        &observations,
    );
    let codex = &controller.sections()[0];

    assert_eq!(codex.label(), "Codex · codex@example.test");
    assert!(codex.choices()[0].is_enabled());
    assert!(codex.choices()[0].is_current());
    assert!(
        controller.sections()[1..]
            .iter()
            .flat_map(yo_core::ModelPickerSection::choices)
            .all(|choice| !choice.is_enabled())
    );
}

// 새 thread가 확인한 exact model은 시작 전에 읽은 catalog default보다 우선합니다. 따라서
// 실제 model은 current/no-op이고 이전 default는 같은 account의 rebind 선택지로 남습니다.
#[test]
fn confirmed_codex_binding_overrides_the_pre_start_catalog_default() {
    let host = HostId::codex();
    let account_label = "codex@example.test";
    let account = derive_host_account_id(&host, &[("email", account_label)]).unwrap();
    let catalog_default = ModelId::new("gpt-catalog-default").unwrap();
    let started_model = ModelId::new("gpt-thread-start").unwrap();
    let revision = derive_host_catalog_revision(
        &host,
        &account,
        Some(&catalog_default),
        &[catalog_default.clone(), started_model.clone()],
    );
    let catalog = HostModelCatalog::new(
        host.clone(),
        "Codex",
        account.clone(),
        account_label,
        revision,
        Some(catalog_default.clone()),
        vec![
            HostCatalogModel::selectable(catalog_default.clone(), catalog_default.to_string())
                .unwrap(),
            HostCatalogModel::selectable(started_model.clone(), started_model.to_string()).unwrap(),
        ],
    )
    .unwrap();
    let observations = vec![HostCatalogObservation::new(host.clone(), Ok(catalog))];

    let active = resolve_active_host_model(
        Some(&host),
        Some((&account, &started_model)),
        true,
        false,
        &observations,
    )
    .unwrap();
    let controller = project_host_catalogs(
        ModelSelectionController::new(ModelCatalog::default(), None),
        Some(&active),
        &observations,
    );
    let choices = controller.sections()[0].choices();

    let previous_default = choices
        .iter()
        .find(|choice| choice.detail() == catalog_default.as_str())
        .unwrap();
    let started = choices
        .iter()
        .find(|choice| choice.detail() == started_model.as_str())
        .unwrap();
    assert!(previous_default.is_enabled());
    assert!(!previous_default.is_current());
    assert!(started.is_enabled());
    assert!(started.is_current());
}

// 한 host를 실행하거나 인증할 수 없어도 Codex와 Grok status section을 각각 남겨, 빈
// 목록처럼 보이거나 sibling failure가 다른 host를 숨기는 회귀를 막습니다.
#[test]
fn unavailable_builtin_hosts_keep_independent_status_sections() {
    let observations = vec![
        HostCatalogObservation::new(HostId::codex(), Err("codex missing".to_owned())),
        HostCatalogObservation::new(HostId::grok(), Err("grok login missing".to_owned())),
    ];

    let controller = project_host_catalogs(
        ModelSelectionController::new(ModelCatalog::default(), None),
        None,
        &observations,
    );
    let sections = controller.sections();

    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].label(), "Codex · local");
    assert_eq!(sections[1].label(), "Grok · local");
    assert!(
        sections
            .iter()
            .all(|section| section.status() == Some(CATALOG_UNAVAILABLE))
    );
}

fn managed_selection() -> ModelSelection {
    ModelSelection::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("qwen3.8-max").unwrap(),
    )
}

fn managed_entry() -> ModelCatalogEntry {
    ModelCatalogEntry::new(
        EffectiveModelBinding::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("default").unwrap(),
            ModelId::new("qwen3.8-max").unwrap(),
            ApiDialect::OpenAiResponses,
            NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        ),
        Some("Qwen Cloud".to_owned()),
        Some("Default".to_owned()),
        Some("Qwen 3.8 Max".to_owned()),
        ModelContextProfile::new(1_000, 100, "utf8-bytes/v1").unwrap(),
    )
    .unwrap()
}

fn host_catalog(host: HostId) -> HostModelCatalog {
    let account_label = format!("{}@example.test", host.as_str());
    let account = derive_host_account_id(&host, &[("email", &account_label)]).unwrap();
    let model = match host.as_str() {
        HostId::CODEX => ModelId::new("gpt-5.6").unwrap(),
        HostId::GROK => ModelId::new("grok-4.6").unwrap(),
        _ => unreachable!("the test constructs only built-in hosts"),
    };
    let revision =
        derive_host_catalog_revision(&host, &account, Some(&model), std::slice::from_ref(&model));
    HostModelCatalog::new(
        host.clone(),
        match host.as_str() {
            HostId::CODEX => "Codex",
            HostId::GROK => "Grok",
            _ => unreachable!("the test constructs only built-in hosts"),
        },
        account,
        account_label,
        revision,
        Some(model.clone()),
        vec![HostCatalogModel::selectable(model.clone(), model.to_string()).unwrap()],
    )
    .unwrap()
}
