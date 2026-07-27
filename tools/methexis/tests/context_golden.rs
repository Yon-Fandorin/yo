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

#[test]
fn direct_request_reproduces_the_exact_context_and_manifest_goldens() {
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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(
        fs::read(root.join(result["context"]["path"].as_str().unwrap())).unwrap(),
        fs::read(examples.join("context.md")).unwrap()
    );
    assert_eq!(
        fs::read(root.join(result["manifest"]["path"].as_str().unwrap())).unwrap(),
        fs::read(examples.join("manifest.json")).unwrap()
    );
}

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

// 독립 decoder는 승인 전환 중에도 golden 후보에서 선택된 정렬 부분집합을 안전하게 읽는다.
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

    assert!(
        affected_ids
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("tui."))
    );
}
