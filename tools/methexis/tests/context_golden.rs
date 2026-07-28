use std::{fs, path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
}

fn methexis() -> Command {
    Command::new(env!("CARGO_BIN_EXE_methexis"))
}

fn assert_artifacts_match(
    root: &Path,
    examples: &Path,
    result: &serde_json::Value,
    context_golden: &str,
    manifest_golden: &str,
) {
    assert_eq!(
        fs::read(root.join(result["context"]["path"].as_str().unwrap())).unwrap(),
        fs::read(examples.join(context_golden)).unwrap()
    );
    assert_eq!(
        fs::read(root.join(result["manifest"]["path"].as_str().unwrap())).unwrap(),
        fs::read(examples.join(manifest_golden)).unwrap()
    );
}

// 현재 저장소에서 필수 의존 지식이 비활성이면 어떤 지식이 막혔는지 실패로 알려 준다.
// 모두 활성이면 완전한 의존성 context와 manifest가 golden을 재현하며, 현재 상태의 한 경로만
// 실행한다.
#[test]
fn direct_request_preserves_the_typed_flow_dependency_closure() {
    let root = repository_root();
    let examples = root.join("tools/methexis/examples/context-contract");
    let output = methexis()
        .current_dir(root)
        .args([
            "resolve-context",
            "tools/methexis/examples/context-contract/direct-request.json",
        ])
        .output()
        .unwrap();

    if output.status.success() {
        assert!(output.stderr.is_empty());
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_artifacts_match(root, &examples, &result, "context.md", "manifest.json");
        return;
    }

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let failure: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(failure["error"]["code"], "required_knowledge_blocked");
    assert_eq!(
        failure["error"]["affected_ids"],
        serde_json::json!([
            "tui.architecture.module-boundaries",
            "tui.runtime.typed-flow"
        ])
    );
}

// 다른 지식의 승인 전환과 무관한 활성 leaf는 계속 사용할 수 있어야 한다.
// 의존성이 없는 이 지식만으로 만든 context와 manifest가 exact golden과 일치하는지 확인한다.
#[test]
fn stable_leaf_request_reproduces_the_exact_context_and_manifest_goldens() {
    let root = repository_root();
    let examples = root.join("tools/methexis/examples/context-contract");
    let output = methexis()
        .current_dir(root)
        .args([
            "resolve-context",
            "tools/methexis/examples/context-contract/stable-leaf-request.json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_artifacts_match(
        root,
        &examples,
        &result,
        "stable-leaf-context.md",
        "stable-leaf-manifest.json",
    );
}

// 지원하지 않는 tokenizer 요청의 실패 출력이 golden과 정확히 일치하는지 검증한다.
#[test]
fn unsupported_tokenizer_reproduces_the_failure_golden() {
    let root = repository_root();
    let output = methexis()
        .current_dir(root)
        .args([
            "resolve-context",
            "tools/methexis/examples/context-contract/unsupported-tokenizer-request.json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let actual: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    let expected: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("tools/methexis/examples/context-contract/unsupported-tokenizer.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(actual, expected);
}

// Methexis가 Librarian의 golden 후보 파일을 독립적으로 검증한 뒤 안전한 지식만 선택하는지 확인한다.
// 선택된 id는 비어 있지 않고 중복 없이 정렬돼야 한다. Manifest의 후보 결정은
// hash-pinned 입력 후보의 ID와 순서를 그대로 보존해 독립 디코더가 입력을 바꾸지 않아야 한다.
#[test]
fn independent_decoder_accepts_the_librarian_contract_golden() {
    let root = repository_root();
    let output = methexis()
        .current_dir(root)
        .args([
            "resolve-context",
            "tools/methexis/examples/context-contract/librarian-request.json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    let affected_ids = result["affected_ids"].as_array().unwrap();
    assert!(!affected_ids.is_empty());
    assert!(
        affected_ids
            .windows(2)
            .all(|pair| { pair[0].as_str().unwrap() < pair[1].as_str().unwrap() })
    );

    let candidates: serde_json::Value = serde_json::from_slice(
        &fs::read(
            root.join("tools/librarian/examples/discovery-contract/expected-query-english.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let candidate_ids: Vec<_> = candidates["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["id"].as_str().unwrap())
        .collect();
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(result["manifest"]["path"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let decision_ids: Vec<_> = manifest["plan"]["candidate_decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|decision| decision["candidate"]["id"].as_str().unwrap())
        .collect();

    assert_eq!(decision_ids, candidate_ids);
}
