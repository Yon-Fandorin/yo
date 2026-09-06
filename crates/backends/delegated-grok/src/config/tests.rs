use std::ffi::OsString;

use super::{
    GrokBackendConfig, NATIVE_SANDBOX_REVIEW_PROFILE, OUTER_SANDBOX_REVIEW_PROFILE,
    REVIEW_RUNNER_CAPABILITIES,
};

// tracked capability manifest는 trusted current-develop runner가 outer isolation을 실제
// claim 전에 지원하는지 도구가 판별하는 작은 machine-readable handshake입니다.
#[test]
fn review_runner_capability_manifest_matches_the_backend_profile() {
    let manifest: serde_json::Value = serde_json::from_slice(REVIEW_RUNNER_CAPABILITIES).unwrap();
    assert_eq!(
        manifest["schema"],
        "yo.delegated-review-runner-capabilities/v1alpha1"
    );
    assert_eq!(manifest["host"], "grok");
    assert_eq!(
        manifest["execution_isolations"],
        serde_json::json!([NATIVE_SANDBOX_REVIEW_PROFILE, OUTER_SANDBOX_REVIEW_PROFILE])
    );
}

// Grok 리뷰 프로필은 읽기 도구만 허용하고 권한 질문·하위 agent·웹 검색을 process
// 시작 시점부터 닫아 ACP event 계층에 도달하기 전에도 같은 제한을 유지합니다.
#[test]
fn read_only_review_uses_the_closed_agent_arguments() {
    let config = GrokBackendConfig::new(".").with_read_only_review(true);

    assert_eq!(
        config.process_arguments(),
        [
            "--sandbox",
            "read-only",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "Read,Grep",
            "--no-subagents",
            "--disable-web-search",
            "agent",
            "stdio",
        ]
        .map(OsString::from)
    );
}

// Yo outer sandbox 리뷰는 Grok 자체 Landlock을 중복 요청하지 않고 tool allow-list를
// 비워, immutable packet 밖의 host 파일을 모델이 조회할 실행 표면을 남기지 않습니다.
#[test]
fn outer_sandbox_review_disables_the_native_profile_and_all_tools() {
    let config = GrokBackendConfig::new(".")
        .with_read_only_review(true)
        .with_outer_sandboxed_review(true);

    assert_eq!(
        config.process_arguments(),
        [
            "--sandbox",
            "off",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
            "--no-subagents",
            "--disable-web-search",
            "agent",
            "stdio",
        ]
        .map(OsString::from)
    );
}

// 일반 host Session은 기존 Grok ACP 진입 argv를 보존합니다.
#[test]
fn standard_profile_preserves_the_existing_agent_arguments() {
    assert_eq!(
        GrokBackendConfig::new(".").process_arguments(),
        ["agent", "stdio"].map(OsString::from)
    );
}
