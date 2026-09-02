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
    let requests = inventory_requests(None, true);

    assert_eq!(requests[0].host, HostId::codex());
    assert_eq!(requests[1].host, HostId::grok());
    assert_eq!(requests[0].execution, DelegatedExecutionProfile::Standard);
    assert_eq!(requests[1].execution, DelegatedExecutionProfile::Standard);
    assert!(!requests[0].outer_sandboxed_review);
    assert!(!requests[1].outer_sandboxed_review);
}

// 제한 실행 프로필과 outer sandbox는 정확한 활성 Grok에만 적용하고, sibling Codex
// inventory는 일반 session-free profile로 남겨 다른 host의 정책을 전이하지 않습니다.
#[test]
fn active_read_only_grok_does_not_change_the_codex_inventory_profile() {
    let grok = HostId::grok();
    let requests = inventory_requests(
        Some((&grok, DelegatedExecutionProfile::ReadOnlyReview)),
        true,
    );

    assert_eq!(requests[0].execution, DelegatedExecutionProfile::Standard);
    assert!(!requests[0].outer_sandboxed_review);
    assert_eq!(
        requests[1].execution,
        DelegatedExecutionProfile::ReadOnlyReview
    );
    assert!(requests[1].outer_sandboxed_review);
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
}

// delegated Grok이 활성인 picker는 Grok exact model만 current로 올리고, managed와 Codex
// section을 sibling으로 유지해 현재 host 하나가 unified catalog를 독점하지 않게 합니다.
#[test]
fn active_host_is_current_while_every_sibling_section_remains_visible() {
    let controller = ModelSelectionController::new(
        ModelCatalog::new(vec![managed_entry()]).unwrap(),
        Some(managed_selection()),
    );
    let observations = vec![
        HostCatalogObservation::new(HostId::codex(), Ok(host_catalog(HostId::codex()))),
        HostCatalogObservation::new(HostId::grok(), Ok(host_catalog(HostId::grok()))),
    ];

    let grok = HostId::grok();
    let controller = project_host_catalogs(controller, Some(&grok), &observations);
    let sections = controller.sections();

    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].label(), "Grok · grok@example.test");
    assert!(sections[0].choices()[0].label().ends_with(" (current)"));
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
