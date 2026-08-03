use std::{ffi::OsString, path::PathBuf, process::ExitCode};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate is nested below the repository root")
        .to_owned()
}

// 실제 Yo SOT 전체를 Librarian 공개 경계로 읽어 구조와 관계를 검증하되, 검색 결과의
// ID·revision을 다시 고정하지 않아 정상적인 SOT 추가와 개정을 허용하는지 확인한다.
#[test]
fn live_repository_sot_is_readable_through_discovery() {
    let root = repository_root();
    let request = root.join("tools/librarian/examples/discovery-contract/query-no-match.json");
    let arguments = vec![
        OsString::from("discover"),
        OsString::from("--repository"),
        root.into_os_string(),
        request.into_os_string(),
    ];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let code = librarian::run(arguments, &mut stdout, &mut stderr).expect("CLI completes");
    assert_eq!(
        code,
        ExitCode::SUCCESS,
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&stdout).expect("candidate-set JSON");
    assert_eq!(result["schema"], "librarian.candidate-set/v1alpha1");
    assert_eq!(result["ok"], true);
}
