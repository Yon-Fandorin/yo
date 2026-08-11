use super::{
    super::model::Effect,
    support::{Fixture, output},
};

// 한 요청이 정확한 develop commit에서 canonical activation 계약, Direct Slice
// branch와 worktree, Git binding을 모두 만들고 생성 경로를 결과에 고정한다.
#[test]
fn creates_and_binds_the_canonical_activation_slice() {
    let fixture = Fixture::new("activation-create");

    let result = fixture.prepare();

    assert_eq!(
        result.base,
        output(&fixture.repository.path, &["rev-parse", "develop"])
    );
    assert_eq!(
        result.branch_ref,
        format!("refs/heads/slice/direct/{}", fixture.slice)
    );
    assert!(matches!(result.effects.contract, Effect::Created));
    assert!(matches!(result.effects.branch, Effect::Created));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
    let contract: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&result.contract_path).unwrap()).unwrap();
    assert_eq!(
        contract["allowed_write_set"],
        serde_json::json!([
            "methexis/active-checkpoint.yaml",
            "methexis/checkpoints/**",
            "tools/methexis/examples/context-contract/manifest.json",
            "tools/methexis/examples/context-contract/stable-leaf-manifest.json"
        ])
    );
    assert_eq!(
        std::fs::read_to_string(result.binding_path).unwrap(),
        format!("{}\n", result.contract_path.display())
    );
}

// 첫 실행이 완성한 exact contract, worktree, branch, binding에 같은 요청을
// 다시 적용하면 아무 대상을 덮어쓰지 않고 모두 reused로 수렴한다.
#[test]
fn exact_retry_reuses_every_prepared_effect() {
    let fixture = Fixture::new("activation-retry");
    fixture.prepare();

    let result = fixture.prepare();

    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.branch, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Reused));
    assert!(matches!(result.effects.binding, Effect::Reused));
}
