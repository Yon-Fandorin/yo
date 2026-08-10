use super::{
    super::{
        AccountId, ModelCatalog, ModelId, ModelSelection, ModelSelectionController, ProviderId,
    },
    support::selection_entry,
};

// picker는 완전한 좌표 순서를 보존하고 bare Model 참조는 현재 Provider와 Account에
// 닫히지만 qualified 참조는 정확한 다른 좌표를 선택할 수 있는지 검증한다.
#[test]
fn selection_controller_orders_bindings_and_resolves_contextual_or_qualified_references() {
    let catalog = ModelCatalog::new(vec![
        selection_entry("qwencloud", "default", "same", "Qwen Cloud", "Default"),
        selection_entry("openrouter", "team", "same", "OpenRouter", "Team"),
        selection_entry("openrouter", "default", "other", "OpenRouter", "Default"),
        selection_entry("openrouter", "default", "same", "OpenRouter", "Default"),
    ])
    .unwrap();
    let current = ModelSelection::new(
        ProviderId::new("openrouter").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("other").unwrap(),
    );
    let controller = ModelSelectionController::new(catalog, Some(current));

    let ordered = controller
        .choices()
        .iter()
        .map(|choice| choice.selection().row_identity())
        .collect::<Vec<_>>();
    assert_eq!(ordered.len(), 4);
    assert!(ordered[0].contains("openrouter"));
    assert!(ordered[0].contains("default"));
    assert!(ordered[3].contains("qwencloud"));

    let selected = controller.resolve_reference("same").unwrap();
    assert_eq!(selected.provider().as_str(), "openrouter");
    assert_eq!(selected.account().as_str(), "default");
    assert_eq!(selected.model().as_str(), "same");
    assert!(controller.resolve_reference("missing").is_err());

    let cross_provider = controller.resolve_reference("qwencloud::same").unwrap();
    assert_eq!(cross_provider.provider().as_str(), "qwencloud");
    assert_eq!(cross_provider.account().as_str(), "default");

    let cross_account = controller
        .resolve_reference("openrouter:team:same")
        .unwrap();
    assert_eq!(cross_account.provider().as_str(), "openrouter");
    assert_eq!(cross_account.account().as_str(), "team");
}

// 현재 namespace가 없는 새 Codex-default 시작에서는 bare ModelId가 catalog 전체에서
// 하나일 때만 성공하고, 중복 ModelId와 Provider 아래 중복 Account는 완전한 좌표와 함께
// 모호하다고 실패하는지 검증한다.
#[test]
fn namespace_free_references_require_a_unique_coordinate_and_report_sorted_candidates() {
    let catalog = ModelCatalog::new(vec![
        selection_entry("qwencloud", "team", "same", "Qwen Cloud", "Team"),
        selection_entry("openrouter", "default", "same", "OpenRouter", "Default"),
        selection_entry("qwencloud", "default", "same", "Qwen Cloud", "Default"),
        selection_entry("qwencloud", "default", "unique", "Qwen Cloud", "Default"),
    ])
    .unwrap();
    let controller = ModelSelectionController::new(catalog, None);

    let unique = controller.resolve_reference("unique").unwrap();
    assert_eq!(unique.provider().as_str(), "qwencloud");
    assert_eq!(unique.account().as_str(), "default");

    let global = controller
        .resolve_reference("same")
        .unwrap_err()
        .to_string();
    let openrouter = global.find("openrouter:default:same").unwrap();
    let qwen_default = global.find("qwencloud:default:same").unwrap();
    let qwen_team = global.find("qwencloud:team:same").unwrap();
    assert!(global.contains("is ambiguous"));
    assert!(openrouter < qwen_default && qwen_default < qwen_team);

    let provider = controller
        .resolve_reference("qwencloud::same")
        .unwrap_err()
        .to_string();
    assert!(provider.contains("qwencloud:default:same"));
    assert!(provider.contains("qwencloud:team:same"));

    let absent = controller
        .resolve_reference("missing")
        .unwrap_err()
        .to_string();
    assert!(absent.contains("is not configured"));
    assert!(absent.contains("openrouter:default:same"));
    assert!(absent.contains("qwencloud:default:unique"));
}

// reference는 separator 우선순위로 파싱하지 않고 catalog가 생성한 모든 표기와 대조하므로
// ModelId의 콜론·슬래시·점은 보존되고 서로 다른 해석이 맞으면 임의 선택하지 않는지
// 검증한다.
#[test]
fn model_reference_preserves_vendor_punctuation_and_rejects_spelling_collisions() {
    let catalog = ModelCatalog::new(vec![
        selection_entry(
            "qwencloud",
            "default",
            "vendor:model/v2.1",
            "Qwen Cloud",
            "Default",
        ),
        selection_entry(
            "literal",
            "default",
            "qwencloud::vendor:model/v2.1",
            "Literal",
            "Default",
        ),
    ])
    .unwrap();
    let controller = ModelSelectionController::new(catalog, None);

    let qualified = controller
        .resolve_reference("qwencloud:default:vendor:model/v2.1")
        .unwrap();
    assert_eq!(qualified.model().as_str(), "vendor:model/v2.1");

    let collision = controller
        .resolve_reference("qwencloud::vendor:model/v2.1")
        .unwrap_err()
        .to_string();
    assert!(collision.contains("is ambiguous"));
    assert!(collision.contains("literal:default:qwencloud::vendor:model/v2.1"));
    assert!(collision.contains("qwencloud:default:vendor:model/v2.1"));
}

// 잘못된 direct reference는 TUI notice에도 노출되므로 붙여 넣은 임의 길이의 입력을
// 오류가 그대로 보관하지 않고, 판별에 필요한 catalog 좌표만 완전하게 제시하는지
// 검증한다.
#[test]
fn model_reference_diagnostic_bounds_rejected_input() {
    let catalog = ModelCatalog::new(vec![selection_entry(
        "qwencloud",
        "default",
        "qwen3.8-max",
        "Qwen Cloud",
        "Default",
    )])
    .unwrap();
    let controller = ModelSelectionController::new(catalog, None);
    let reference = format!("{}sensitive-tail", "x".repeat(10_000));

    let diagnostic = controller
        .resolve_reference(&reference)
        .unwrap_err()
        .to_string();

    assert!(diagnostic.contains("(truncated) is not configured"));
    assert!(!diagnostic.contains("sensitive-tail"));
    assert!(diagnostic.len() < 1_000);
    assert!(diagnostic.contains("qwencloud:default:qwen3.8-max"));
}
