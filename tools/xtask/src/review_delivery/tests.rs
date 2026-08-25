#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use super::{
    combine_failures, managed_model_reference,
    process::{execute_once, execute_once_with_timeout},
    publish_claim, read_request, require_empty_directory, require_integration_state,
    require_original_fresh,
    session::provider_request_identity,
};
use crate::{
    review_egress::AuthorizedDelivery, review_protocol::digest, test_support::TestRepository,
};

fn authorized() -> AuthorizedDelivery {
    AuthorizedDelivery {
        request_id: "sha256:request".to_owned(),
        authorization_id: "sha256:authorization".to_owned(),
        authority: "human/yon".to_owned(),
        review_kind: "original",
        review_id: "sha256:review".to_owned(),
        candidate_commit: "11".repeat(20),
        trusted_commit: "22".repeat(20),
        packet_hash: "sha256:packet".to_owned(),
        packet_bytes: b"review packet".to_vec(),
        managed_payload_tokens: 3,
        provider: "qwencloud".to_owned(),
        account: "default".to_owned(),
        model: "qwen3.8-max".to_owned(),
        fresh_session: true,
    }
}

// 새 delivery wire는 저장소 규칙대로 v1alpha1에서 시작하고, 비슷한 stable v1이나
// 알 수 없는 필드가 같은 effect를 우회해 실행 요청으로 해석되지 않게 합니다.
#[test]
fn request_requires_the_exact_v1alpha1_shape() {
    let repository = TestRepository::new("review-delivery-request");
    let valid = serde_json::json!({
        "schema": "yo.slice-review-delivery-request/v1alpha1",
        "egress_request_path": ".local-exclude/egress.json",
        "egress_request_hash": digest(b"egress"),
        "output_directory": ".local-exclude/coordination/slice/run"
    });
    let valid_path = repository.write("valid.json", &format!("{valid}\n"));
    read_request(&valid_path).unwrap();

    let stable = valid
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    let mut stable = serde_json::Value::Object(stable);
    stable["schema"] = "yo.slice-review-delivery-request/v1".into();
    let stable_path = repository.write("stable.json", &format!("{stable}\n"));
    assert!(read_request(&stable_path).unwrap_err().contains("v1alpha1"));

    let mut extra = valid;
    extra["retry"] = 1.into();
    let extra_path = repository.write("extra.json", &format!("{extra}\n"));
    assert!(
        read_request(&extra_path)
            .unwrap_err()
            .contains("unknown field")
    );
}

// 첫 protocol은 원본 packet의 fresh Session만 실행하여 기존 resume 권한이 print
// continuation 구현으로 조용히 확대되지 않게 합니다.
#[test]
fn first_delivery_protocol_rejects_resume_and_delta() {
    let mut delivery = authorized();
    require_original_fresh(&delivery).unwrap();

    delivery.fresh_session = false;
    assert!(
        require_original_fresh(&delivery)
            .unwrap_err()
            .contains("original")
    );
    delivery.fresh_session = true;
    delivery.review_kind = "finding_resolution";
    assert!(
        require_original_fresh(&delivery)
            .unwrap_err()
            .contains("fresh")
    );
}

// managed model reference의 `:` 구분자를 route 구성요소가 포함하면 다른 target으로
// 재해석될 수 있으므로 claim 전에 거부하고 exact Provider/Account/Model을 보존합니다.
#[test]
fn managed_model_reference_rejects_ambiguous_components() {
    let delivery = authorized();
    assert_eq!(
        managed_model_reference(&delivery).unwrap(),
        "qwencloud:default:qwen3.8-max"
    );
    let mut ambiguous = delivery;
    ambiguous.account = "default:other".to_owned();
    assert!(
        managed_model_reference(&ambiguous)
            .unwrap_err()
            .contains('`')
    );
}

// 실행 전 output directory가 비어 있어야 claim과 결과가 이전 attempt의 파일 위에
// 겹치지 않고, 첫 파일이 생긴 순간 같은 경로의 재사용을 거부합니다.
#[test]
fn output_directory_must_be_empty_before_claim() {
    let repository = TestRepository::new("review-delivery-output");
    let output = repository.path.join("output");
    std::fs::create_dir(&output).unwrap();
    require_empty_directory(&output).unwrap();
    std::fs::write(output.join("claim.json"), b"claimed").unwrap();
    assert!(
        require_empty_directory(&output)
            .unwrap_err()
            .contains("must be empty")
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
    std::fs::create_dir(&output).unwrap();
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
    std::fs::create_dir(&output).unwrap();
    let executable = repository.write("yo", "#!/bin/sh\nwhile :; do :; done\n");
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();
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

// 결과 identity가 생략된 유효한 durable outcome은 accepted request identity로
// 귀결하고, identity 유무와 관계없이 outcome record가 둘이면 exact-one을 거부합니다.
#[test]
fn provider_request_identity_counts_outcomes_and_uses_the_accepted_fallback() {
    let requests = vec!["request-1".to_owned()];
    assert_eq!(
        provider_request_identity(&requests, &[None]).unwrap(),
        "request-1"
    );
    assert_eq!(
        provider_request_identity(&requests, &[Some("outcome-1".to_owned())]).unwrap(),
        "outcome-1"
    );
    assert!(
        provider_request_identity(&requests, &[Some("outcome-1".to_owned()), None])
            .unwrap_err()
            .contains("2 resumable outcomes")
    );
}

// claim 이후 temporary capture 파일을 만들 수 없어도 실행 경계는 Err로 빠져나가지
// 않고 bounded failed capture를 돌려 상위가 outcome.json을 게시할 기회를 보존합니다.
#[test]
fn post_claim_capture_setup_failure_remains_a_bounded_capture() {
    let repository = TestRepository::new("review-delivery-capture-setup");
    let output = repository.path.join("output");
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join(".review.stdout.tmp"), b"occupied").unwrap();

    let capture = execute_once(
        &repository.path.join("must-not-start"),
        &repository.path,
        &output,
        "qwencloud:default:qwen3.8-max",
        &authorized(),
    );

    assert!(capture.status.is_none());
    assert!(capture.stdout.is_empty());
    assert!(capture.stderr.is_empty());
    assert!(
        capture
            .failure
            .as_deref()
            .unwrap()
            .contains("cannot create")
    );
}

// develop build의 exactness 검사에는 tracked 변경뿐 아니라 build resolution을 바꿀 수
// 있는 untracked 파일도 포함되어야 하며, 생긴 즉시 claim 전에 거부합니다.
#[test]
fn integration_state_rejects_untracked_files() {
    let repository = TestRepository::new("review-delivery-integration-clean");
    repository.write("tracked.txt", "tracked\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "test: base"]);
    let head = crate::git::trusted_output_in(&repository.path, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    require_integration_state(&repository.path, &head).unwrap();

    repository.write("untracked.txt", "can affect resolution\n");
    assert!(
        require_integration_state(&repository.path, &head)
            .unwrap_err()
            .contains("must be clean")
    );
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
