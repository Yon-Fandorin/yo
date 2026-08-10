use super::{
    super::{
        AccountId, ModelCatalog, ModelId, ModelSelection, ModelSelectionController, ProviderId,
        StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target,
    },
    support::selection_entry,
};

// HostTarget은 bare model namespace와 분리되고, 같은 bytes의 ModelId는 qualified form으로만
// 선택되어 Local Codex identity를 shadow하지 않는다.
#[test]
fn startup_target_reserves_exact_host_codex_without_hiding_a_qualified_model() {
    let catalog = ModelCatalog::new(vec![selection_entry(
        "manual",
        "default",
        "host:codex",
        "Manual",
        "Default",
    )])
    .unwrap();
    let controller = ModelSelectionController::new(catalog, None);

    assert_eq!(
        controller.resolve_target_reference("host:codex").unwrap(),
        StartupTarget::HostCodex
    );
    let model = controller
        .resolve_target_reference("manual::host:codex")
        .unwrap();
    assert_eq!(model.model().unwrap().model().as_str(), "host:codex");
}

// Provider와 Account의 예약 문자는 canonical uppercase escape로만 좌표에 참여하고 Model
// suffix의 vendor punctuation은 그대로 보존한다.
#[test]
fn target_reference_uses_canonical_coordinate_escaping() {
    let catalog = ModelCatalog::new(vec![selection_entry(
        "vendor:edge",
        "team%blue",
        "model:latest/v1",
        "Vendor",
        "Team",
    )])
    .unwrap();
    let controller = ModelSelectionController::new(catalog, None);

    let selected = controller
        .resolve_target_reference("vendor%3Aedge:team%25blue:model:latest/v1")
        .unwrap();
    assert_eq!(selected.model().unwrap().provider().as_str(), "vendor:edge");
    assert!(
        controller
            .resolve_target_reference("vendor%3aedge:team%25blue:model:latest/v1")
            .is_err()
    );
    assert!(
        controller
            .resolve_target_reference("vendor%3Aedge:team%blue:model:latest/v1")
            .is_err()
    );
}

// Overridable policy는 invocation, stored preference, policy default, operator target 순서를
// 지키며 모든 source가 없을 때 Local Codex를 발명하지 않는다.
#[test]
fn startup_resolution_uses_declared_precedence_and_has_no_implicit_target() {
    let catalog = ModelCatalog::new(vec![
        selection_entry("p", "a", "operator", "P", "A"),
        selection_entry("p", "a", "policy", "P", "A"),
        selection_entry("p", "a", "stored", "P", "A"),
        selection_entry("p", "a", "invoked", "P", "A"),
    ])
    .unwrap();
    let target = |model: &str| {
        StartupTarget::Model(ModelSelection::new(
            ProviderId::new("p").unwrap(),
            AccountId::new("a").unwrap(),
            ModelId::new(model).unwrap(),
        ))
    };
    let policy = StartupPolicy::new(true, None, Some(target("policy"))).unwrap();

    let selected = resolve_startup_target(
        &catalog,
        &policy,
        StartupSelectionSources {
            invocation: Some("invoked"),
            stored_preference: Some(target("stored")),
            operator_target: Some(target("operator")),
        },
    )
    .unwrap();
    assert_eq!(selected, Some(target("invoked")));

    let stored = resolve_startup_target(
        &catalog,
        &policy,
        StartupSelectionSources {
            stored_preference: Some(target("stored")),
            operator_target: Some(target("operator")),
            ..StartupSelectionSources::default()
        },
    )
    .unwrap();
    assert_eq!(stored, Some(target("stored")));
    assert_eq!(
        resolve_startup_target(
            &ModelCatalog::default(),
            &StartupPolicy::initial(),
            StartupSelectionSources::default(),
        )
        .unwrap(),
        None
    );
}

// Enforced policy만 invocation 충돌을 거절하고, malformed field 조합은 capture 경계에서
// 선택 우선순위와 무관하게 실패한다.
#[test]
fn enforced_startup_policy_rejects_conflicts_and_malformed_forms() {
    let catalog = ModelCatalog::new(vec![
        selection_entry("p", "a", "same", "P", "A"),
        selection_entry("p", "b", "same", "P", "B"),
    ])
    .unwrap();
    let enforced_model = StartupTarget::Model(ModelSelection::new(
        ProviderId::new("p").unwrap(),
        AccountId::new("a").unwrap(),
        ModelId::new("same").unwrap(),
    ));
    let policy = StartupPolicy::new(false, Some(enforced_model.clone()), None).unwrap();
    assert_eq!(
        resolve_startup_target(
            &catalog,
            &policy,
            StartupSelectionSources {
                invocation: Some("same"),
                stored_preference: Some(StartupTarget::Model(ModelSelection::new(
                    ProviderId::new("p").unwrap(),
                    AccountId::new("b").unwrap(),
                    ModelId::new("same").unwrap(),
                ))),
                ..StartupSelectionSources::default()
            },
        )
        .unwrap(),
        Some(enforced_model)
    );
    assert!(
        resolve_startup_target(
            &catalog,
            &policy,
            StartupSelectionSources {
                invocation: Some("host:codex"),
                ..StartupSelectionSources::default()
            },
        )
        .is_err()
    );
    assert!(StartupPolicy::new(false, None, None).is_err());
    assert!(
        StartupPolicy::new(
            false,
            Some(StartupTarget::HostCodex),
            Some(StartupTarget::HostCodex),
        )
        .is_err()
    );
}
