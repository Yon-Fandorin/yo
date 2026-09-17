use super::*;

// frozen v1alpha1 claim은 새 optional 필드가 None일 때 기존 byte shape를 그대로 유지해
// admission 도입이 과거 artifact 재현을 바꾸지 않습니다.
#[test]
fn v1alpha1_claim_omits_v1alpha2_admission_fields() {
    let claim = Claim {
        schema: CLAIM_SCHEMA,
        request_id: "sha256:request",
        authorization_id: "sha256:authorization",
        authority: "human/yon",
        review_id: "sha256:review",
        candidate_commit: "candidate",
        integration_commit: "integration",
        packet_hash: "sha256:packet",
        packet_bytes: 10,
        managed_payload_tokens: 3,
        route: Route {
            provider: "qwencloud",
            account: "default",
            model: "qwen3.8-max",
        },
        session_mode: "fresh",
        provider_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        second_provider: false,
        tool_execution: false,
        yo_binary_hash: "sha256:binary",
        admission_request_id: None,
        target: None,
    };
    let value: serde_json::Value =
        serde_json::from_slice(&canonical_json(&claim).unwrap()).unwrap();
    assert!(value.get("admission_request_id").is_none());
    assert!(value.get("target").is_none());
}

// 새 usage artifact 필드는 None일 때 frozen result JSON에 나타나지 않고, 새 result
// version이 exact artifact를 제공할 때만 content-addressed reference로 추가됩니다.
#[test]
fn frozen_results_omit_the_new_provider_usage_artifact() {
    let artifact = |path: &str| Artifact {
        path: path.to_owned(),
        hash: digest(path.as_bytes()),
        bytes: path.len(),
        published: true,
    };
    let mut result = ResultDocument {
        schema: "yo.slice-review-delivery-result/v1alpha2",
        ok: true,
        status: "completed",
        next_action: "interpret_review",
        request_id: "request".to_owned(),
        review_id: "review".to_owned(),
        candidate_commit: "11".repeat(20),
        integration_commit: "22".repeat(20),
        session_id: "session".to_owned(),
        provider_request_id: "provider-request".to_owned(),
        review_result: artifact("review.txt"),
        diagnostic: artifact("diagnostic.txt"),
        outcome: artifact("outcome.json"),
        delivery_receipt: artifact("delivery.json"),
        provider_usage: None,
    };
    let value: serde_json::Value =
        serde_json::from_slice(&canonical_json(&result).unwrap()).unwrap();
    assert!(value.get("provider_usage").is_none());

    result.schema = "yo.slice-review-delivery-result/v1alpha3";
    result.provider_usage = Some(artifact("provider-usage.json"));
    let value: serde_json::Value =
        serde_json::from_slice(&canonical_json(&result).unwrap()).unwrap();
    assert_eq!(
        value["provider_usage"]["path"],
        serde_json::json!("provider-usage.json")
    );
}

// claim은 최초 한 번만 새 파일로 게시되고 같은 bytes라도 재호출을 성공으로 재사용하지
// 않아, crash 뒤의 자동 재실행이 두 번째 Provider request로 이어지지 않게 합니다.
#[test]
fn exact_claim_cannot_be_reused_as_resend_authority() {
    let repository = TestRepository::new("review-delivery-claim");
    let claim = repository.path.join("claim.json");
    publish_claim(&claim, b"claim\n").unwrap();
    assert!(
        publish_claim(&claim, b"claim\n")
            .unwrap_err()
            .contains("refusing another provider request")
    );
}

// claim 뒤 process spawn 자체가 실패해도 panic이나 두 번째 launch 없이 bounded capture로
// 돌아와 호출자가 compact failed outcome을 게시할 수 있게 합니다.
#[test]
fn claimed_spawn_failure_returns_one_bounded_capture() {
    let repository = TestRepository::new("review-delivery-spawn-failure");
    let output = repository.path.join("output");
    fs::create_dir(&output).unwrap();
    let capture = execute_once(
        &repository.path.join("missing-yo"),
        &repository.path,
        &output,
        "qwencloud:default:qwen3.8-max",
        &authorized(),
    );

    assert!(capture.status.is_none());
    assert!(capture.stdout.is_empty());
    assert!(capture.stderr.is_empty());
    assert!(capture.failure.as_deref().unwrap().contains("cannot start"));
    assert!(!output.join(".review.stdout.tmp").exists());
    assert!(!output.join(".review.stderr.tmp").exists());
}

#[cfg(unix)]
// Provider process가 종료되지 않아도 fixed deadline 뒤에는 자식을 종료·회수하고
// 한 failed capture로 돌아와 coordinator가 같은 claim 아래에서 무한 대기하지 않습니다.
#[test]
fn claimed_process_is_terminated_at_its_deadline() {
    let repository = TestRepository::new("review-delivery-timeout");
    let output = repository.path.join("output");
    fs::create_dir(&output).unwrap();
    let executable = repository.write("yo", "#!/bin/sh\nwhile :; do :; done\n");
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).unwrap();
    let started = Instant::now();

    let capture = execute_once_with_timeout(
        &executable,
        &repository.path,
        &output,
        "qwencloud:default:qwen3.8-max",
        &authorized(),
        Duration::from_millis(50),
    );

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(capture.status.is_none());
    assert!(capture.failure.as_deref().unwrap().contains("exceeded its"));
}

// process failure와 durable observation failure가 함께 생기면 어느 하나도 버리지 않고
// 한 bounded outcome 진단으로 합쳐 중복 전송 판단에 필요한 원인을 보존합니다.
#[test]
fn outcome_retains_process_and_observation_failures() {
    assert_eq!(combine_failures(None, None), None);
    assert_eq!(
        combine_failures(Some("process".to_owned()), Some("session".to_owned())),
        Some("process; session".to_owned())
    );
}
