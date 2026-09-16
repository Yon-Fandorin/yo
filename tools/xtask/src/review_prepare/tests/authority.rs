use std::{fs, path::PathBuf, str};

use super::super::authority::{
    authority_paths_for_changed_paths_v1alpha1, authority_paths_for_changed_paths_v1alpha2,
};

// code-only 후보는 큰 repository workflow 문서를 반복 복사하지 않고, workflow 구현이나
// AGENTS authority 자체가 바뀐 후보만 exact authority bytes를 packet에 포함합니다.
#[test]
fn changed_authority_policy_omits_fixed_cost_for_product_code() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha1(&["crates/yo-core/src/lib.rs".to_owned()]),
        vec!["AGENTS.md".to_owned()]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha1(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "nested/AGENTS.md".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING.md".to_owned(),
            "nested/AGENTS.md".to_owned()
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha1(&[
            "CONTRIBUTING/review-and-integration.md".to_owned(),
        ]),
        vec!["AGENTS.md".to_owned(), "CONTRIBUTING.md".to_owned()]
    );
}

// alpha2 policy는 변경된 workflow 책임만 직접 소유 문서로 라우팅하고, 공용 facade는
// 구체 구현 경로가 정한 소유자를 불필요하게 넓히지 않습니다.
#[test]
fn changed_authority_policy_v1alpha2_routes_precise_workflow_owners() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "tools/xtask/src/review_prepare.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned()
        ]
    );
    for path in [
        "tools/xtask/src/review_delivery.rs",
        "tools/xtask/src/review_delivery/admission.rs",
        "tools/xtask/src/review_delivery/artifact.rs",
        "tools/xtask/src/review_delivery/original.rs",
        "tools/xtask/src/review_delivery/continuation.rs",
        "tools/xtask/src/review_delivery/workspace.rs",
    ] {
        assert_eq!(
            authority_paths_for_changed_paths_v1alpha2(&[
                "tools/xtask/src/lib.rs".to_owned(),
                path.to_owned(),
            ]),
            vec![
                "AGENTS.md".to_owned(),
                "CONTRIBUTING/review-delivery.md".to_owned()
            ]
        );
    }
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "tools/xtask/src/slice_gate/mod.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/slice_contract/mod.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/formal-slices.md".to_owned()
        ]
    );
}

// 여러 책임을 실제로 건드린 후보는 소유 문서의 합집합을 포함하고, 변경된 nested
// AGENTS authority도 exact path로 보존합니다.
#[test]
fn changed_authority_policy_v1alpha2_unions_cross_owner_changes() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/review_packet/mod.rs".to_owned(),
            "tools/xtask/src/review_egress/mod.rs".to_owned(),
            "tools/xtask/src/slice_close/mod.rs".to_owned(),
            "nested/AGENTS.md".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned(),
            "nested/AGENTS.md".to_owned(),
        ]
    );
}

// packet, delivery, gate가 함께 소비하는 protocol과 packet/gate가 함께 소비하는 result
// 파일은 한 소유자로 축소하지 않고 실제 소비 영역의 authority 합집합을 포함합니다.
#[test]
fn changed_authority_policy_v1alpha2_routes_shared_protocol_owners() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/review_protocol.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned(),
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/review_result.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned(),
        ]
    );
}

// 소유자를 식별할 companion 없이 공용 facade나 shared workflow 기반만 바뀌면 모든
// workflow owner를 포함해 누락 대신 비용 증가로 fail-closed합니다.
#[test]
fn changed_authority_policy_v1alpha2_fails_closed_for_ambiguous_workflow() {
    let expected = vec![
        "AGENTS.md".to_owned(),
        "CONTRIBUTING.md".to_owned(),
        "CONTRIBUTING/formal-slices.md".to_owned(),
        "CONTRIBUTING/review-and-integration.md".to_owned(),
        "CONTRIBUTING/review-delivery.md".to_owned(),
        "CONTRIBUTING/review-packets.md".to_owned(),
    ];
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&["tools/xtask/src/lib.rs".to_owned()]),
        expected
    );
    for path in [
        "tools/xtask/src/cli.rs",
        "tools/xtask/src/cli/check.rs",
        "tools/xtask/src/cli/docs.rs",
        "tools/xtask/src/cli/slice.rs",
        "tools/xtask/src/cli/slice/lifecycle.rs",
        "tools/xtask/src/cli/slice/review.rs",
        "tools/xtask/src/cli/tests/mod.rs",
    ] {
        assert_eq!(
            authority_paths_for_changed_paths_v1alpha2(&[path.to_owned()]),
            expected,
            "neutral CLI facade must retain the fail-closed authority set for {path}"
        );
    }
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/bounded_file.rs".to_owned(),
            "tools/xtask/src/review_prepare/orchestration.rs".to_owned(),
        ]),
        expected
    );
}

// 기본 session context에 절대 token 상한을 적용하여 다른 문서를 늘리는 방식으로
// 절약 수치를 만족시키지 못하게 한다. Formal 상세 문서는 기본 읽기에서 분리된다.
#[test]
fn default_workflow_has_an_absolute_context_budget() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = fs::read(repository.join("CONTRIBUTING.md")).unwrap();
    let index = fs::read(repository.join("AGENTS.md")).unwrap();
    let tokenizer = tiktoken_rs::o200k_base_singleton();
    let count = |bytes: &[u8]| {
        tokenizer
            .encode_with_special_tokens(str::from_utf8(bytes).unwrap())
            .len()
    };
    let root_tokens = count(&root);
    assert!(
        root_tokens <= 1_300,
        "default workflow uses {root_tokens} tokens"
    );
    let startup_tokens = root_tokens + count(&index);
    eprintln!("workflow context: root={root_tokens}, startup={startup_tokens}");
    assert!(
        startup_tokens <= 1_700,
        "default startup uses {startup_tokens} tokens"
    );
    let formal = fs::read(repository.join("CONTRIBUTING/formal-slices.md")).unwrap();
    let formal_tokens = count(&formal);
    eprintln!("formal workflow context: {formal_tokens}");
    assert!(
        formal_tokens <= 1_800,
        "formal workflow uses {formal_tokens} tokens"
    );
}

// 일반 hook은 짧은 기본 규칙만 읽고, formal 문서 변경은 이동된 실제 owner를
// 포함해야 한다. 단순 코드 변경에 거대한 formal 절차를 덧붙이지 않는다.
#[test]
fn ordinary_and_formal_workflows_route_to_their_actual_owners() {
    for (path, owner) in [
        ("tools/xtask/src/impact/change.rs", "CONTRIBUTING.md"),
        ("tools/context.py", "CONTRIBUTING.md"),
        ("tools/test_context.py", "CONTRIBUTING.md"),
        (
            "CONTRIBUTING/formal-slices.md",
            "CONTRIBUTING/formal-slices.md",
        ),
    ] {
        assert_eq!(
            authority_paths_for_changed_paths_v1alpha2(&[path.to_owned()]),
            vec!["AGENTS.md".to_owned(), owner.to_owned()]
        );
    }
}

// HK가 직접 실행하는 helper와 외부 검증기는 실제 workflow 책임으로만 연결하여,
// 다른 review owner를 모두 읽게 하는 ambiguous fallback을 피합니다.
#[test]
fn executable_helpers_route_to_their_direct_owners() {
    for (path, owner) in [
        ("tools/chat_preview.py", "CONTRIBUTING.md"),
        ("tools/test_chat_preview.py", "CONTRIBUTING.md"),
        ("tools/clipboard_bridge.py", "CONTRIBUTING.md"),
        ("tools/test_clipboard_bridge.py", "CONTRIBUTING.md"),
        ("tools/test_ssh_clipboard_capture.py", "CONTRIBUTING.md"),
        (
            "crates/yo-cli/src/execution/image/clipboard/ssh_capture.py",
            "CONTRIBUTING.md",
        ),
        (
            "tools/validation/developer-docs-build.sh",
            "CONTRIBUTING.md",
        ),
        ("tools/validation/developer-docs.sh", "CONTRIBUTING.md"),
        (
            "tools/validation/developer-docs-translations-tests.sh",
            "CONTRIBUTING.md",
        ),
        (
            "tools/validation/developer-docs-translations.sh",
            "CONTRIBUTING.md",
        ),
        (
            "tools/validation/yo-cli-unix-matrix-tests.sh",
            "CONTRIBUTING.md",
        ),
        ("tools/validation/yo-cli-unix-matrix.sh", "CONTRIBUTING.md"),
        (
            "tools/validation/codex-interview-resume.py",
            "CONTRIBUTING/review-packets.md",
        ),
        (
            "tools/validation/codex-policy-persistence.py",
            "CONTRIBUTING/review-packets.md",
        ),
        (
            "tools/validation/managed-start-failure.py",
            "CONTRIBUTING/review-packets.md",
        ),
    ] {
        assert_eq!(
            authority_paths_for_changed_paths_v1alpha2(&[path.to_owned()]),
            vec!["AGENTS.md".to_owned(), owner.to_owned()]
        );
    }
}

// product-only 후보는 기존처럼 작은 root router만 포함하며, 직접 변경한 review owner는
// root CONTRIBUTING을 경유하지 않고 그 exact file을 포함합니다.
#[test]
fn changed_authority_policy_v1alpha2_keeps_product_cost_minimal() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&["crates/yo-core/src/lib.rs".to_owned()]),
        vec!["AGENTS.md".to_owned()]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(
            &["CONTRIBUTING/review-delivery.md".to_owned(),]
        ),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned()
        ]
    );
}
