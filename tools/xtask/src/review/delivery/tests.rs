#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
use std::time::{Duration, Instant};

use super::{
    canonical_json, combine_failures,
    delegated::require_continuation_isolation,
    managed_model_reference,
    model::{Artifact, CLAIM_SCHEMA, Claim, DeliveryRequest, ResultDocument, Route},
    prepare_output_directory_at,
    process::{
        execute_continuation_once, execute_delegated_continuation_once, execute_delegated_once,
        execute_once, execute_once_with_timeout,
    },
    publish_claim, read_request, read_request_with_output_policy, require_empty_directory,
    require_integration_state, require_original_fresh,
};
use crate::{
    review::{
        egress::{AuthorizedDelivery, AuthorizedHostDelivery},
        session::provider_request_identity,
    },
    review_protocol::digest,
    test_support::TestRepository,
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
        session_id: None,
        prior_packet_hash: None,
        prior_provider_request_id: None,
    }
}

fn authorized_host() -> AuthorizedHostDelivery {
    AuthorizedHostDelivery {
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
        host: "codex".to_owned(),
        execution_profile: "yo.delegated-review-execution/v1alpha1".to_owned(),
        fresh_session: true,
        session_id: None,
        prior_packet_hash: None,
        prior_host_request_id: None,
        prior_execution_isolation: None,
    }
}

// continuation은 직전 receipt의 물리적 isolation과 새 admission 결과가 exact하게
// 같을 때만 진행해 같은 reviewer Session의 보안 경계가 중간에 바뀌지 않게 합니다.
#[test]
fn delegated_continuation_pins_prior_execution_isolation() {
    let mut authorized = authorized_host();
    authorized.host = "grok".to_owned();
    authorized.prior_execution_isolation =
        Some(crate::grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE.to_owned());

    require_continuation_isolation(
        &authorized,
        Some(crate::grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
    )
    .unwrap();
    assert!(
        require_continuation_isolation(
            &authorized,
            Some(crate::grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE),
        )
        .unwrap_err()
        .contains("exact prior physical isolation")
    );

    authorized.prior_execution_isolation = None;
    assert!(
        require_continuation_isolation(
            &authorized,
            Some(crate::grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
        )
        .is_err()
    );
}

// continuation effect는 기존 original-fresh wire를 재해석하지 않고 preflight bytes만
// 가리키는 별도 v1alpha1 closed shape로 시작합니다.
#[test]
fn continuation_request_binds_one_preflight_and_output_directory() {
    let repository = TestRepository::new("review-continuation-delivery-request");
    let valid = serde_json::json!({
        "schema": "yo.slice-review-continuation-delivery-request/v1alpha1",
        "preflight_request_path": ".local-exclude/preflight.json",
        "preflight_request_hash": digest(b"preflight"),
        "output_directory": ".local-exclude/coordination/slice/continuation"
    });
    let valid_path = repository.write("valid.json", &format!("{valid}\n"));
    assert!(matches!(
        read_request(&valid_path).unwrap(),
        DeliveryRequest::Continuation(_)
    ));

    let mut extra = valid;
    extra["retry"] = 1.into();
    let extra_path = repository.write("extra.json", &format!("{extra}\n"));
    assert!(
        read_request(&extra_path)
            .unwrap_err()
            .contains("unknown field")
    );
}

// admission-aware delivery는 frozen v1alpha1을 재해석하지 않고 exact admission request
// path와 hash를 추가한 v1alpha2에서만 열립니다.
#[test]
fn v1alpha2_delivery_binds_one_target_admission_request() {
    let repository = TestRepository::new("review-delivery-admission-request");
    let original = serde_json::json!({
        "schema": "yo.slice-review-delivery-request/v1alpha2",
        "egress_request_path": ".local-exclude/egress.json",
        "egress_request_hash": digest(b"egress"),
        "admission_request_path": ".local-exclude/admission.json",
        "admission_request_hash": digest(b"admission"),
        "output_directory": ".local-exclude/coordination/slice/run"
    });
    let path = repository.write("original.json", &format!("{original}\n"));
    assert!(matches!(
        read_request(&path).unwrap(),
        DeliveryRequest::AdmittedOriginal(_)
    ));

    let continuation = serde_json::json!({
        "schema": "yo.slice-review-continuation-delivery-request/v1alpha2",
        "preflight_request_path": ".local-exclude/preflight.json",
        "preflight_request_hash": digest(b"preflight"),
        "admission_request_path": ".local-exclude/admission.json",
        "admission_request_hash": digest(b"admission"),
        "output_directory": ".local-exclude/coordination/slice/continuation"
    });
    let path = repository.write("continuation.json", &format!("{continuation}\n"));
    assert!(matches!(
        read_request(&path).unwrap(),
        DeliveryRequest::AdmittedContinuation(_)
    ));
}

// 문서의 managed alpha3 원본·continuation 형태가 exact admission을 빠뜨리지 않고
// 파싱되며, frozen alpha2와 달리 새 output 준비만 선택함을 확인합니다.
#[test]
fn documented_managed_v1alpha3_shapes_bind_admission_and_prepare_output() {
    let repository = TestRepository::new("review-delivery-output-version");
    let request = |schema: &str| {
        serde_json::json!({
            "schema": schema,
            "egress_request_path": ".local-exclude/egress.json",
            "egress_request_hash": digest(b"egress"),
            "admission_request_path": ".local-exclude/admission.json",
            "admission_request_hash": digest(b"admission"),
            "output_directory": ".local-exclude/coordination/slice/run"
        })
    };
    let alpha2 = repository.write(
        "alpha2.json",
        &format!("{}\n", request("yo.slice-review-delivery-request/v1alpha2")),
    );
    let alpha3 = repository.write(
        "alpha3.json",
        &format!("{}\n", request("yo.slice-review-delivery-request/v1alpha3")),
    );

    assert!(
        !read_request_with_output_policy(&alpha2)
            .unwrap()
            .1
            .prepare_output
    );
    let (request, policy) = read_request_with_output_policy(&alpha3).unwrap();
    assert!(policy.prepare_output);
    assert!(!policy.bind_usage);
    let DeliveryRequest::AdmittedOriginal(request) = request else {
        panic!("alpha3 selected another delivery protocol");
    };
    assert_eq!(request.schema, "yo.slice-review-delivery-request/v1alpha2");

    let continuation = serde_json::json!({
        "schema": "yo.slice-review-continuation-delivery-request/v1alpha3",
        "preflight_request_path": ".local-exclude/preflight.json",
        "preflight_request_hash": digest(b"preflight"),
        "admission_request_path": ".local-exclude/admission.json",
        "admission_request_hash": digest(b"admission"),
        "output_directory": ".local-exclude/coordination/slice/continuation"
    });
    let continuation = repository.write("continuation-alpha3.json", &format!("{continuation}\n"));
    let (request, policy) = read_request_with_output_policy(&continuation).unwrap();
    assert!(policy.prepare_output);
    assert!(!policy.bind_usage);
    let DeliveryRequest::AdmittedContinuation(request) = request else {
        panic!("alpha3 selected another continuation protocol");
    };
    assert_eq!(
        request.schema,
        "yo.slice-review-continuation-delivery-request/v1alpha2"
    );
}

// 기존 alpha3 output 준비 의미를 바꾸지 않고 새 alpha4만 exact request-turn Usage
// artifact를 요구하며, managed와 delegated 요청 모두 같은 opt-in 정책을 선택합니다.
#[test]
fn v1alpha4_requests_opt_into_provider_usage_binding() {
    let repository = TestRepository::new("review-delivery-usage-version");
    let managed = serde_json::json!({
        "schema": "yo.slice-review-delivery-request/v1alpha4",
        "egress_request_path": ".local-exclude/egress.json",
        "egress_request_hash": digest(b"egress"),
        "admission_request_path": ".local-exclude/admission.json",
        "admission_request_hash": digest(b"admission"),
        "output_directory": ".local-exclude/coordination/slice/run"
    });
    let path = repository.write("managed-alpha4.json", &format!("{managed}\n"));
    let (request, policy) = read_request_with_output_policy(&path).unwrap();
    assert!(matches!(request, DeliveryRequest::AdmittedOriginal(_)));
    assert!(policy.prepare_output);
    assert!(policy.bind_usage);

    let delegated = serde_json::json!({
        "schema": "yo.slice-review-delegated-delivery-request/v1alpha4",
        "egress_request_path": ".local-exclude/egress.json",
        "egress_request_hash": digest(b"egress"),
        "admission_request_path": ".local-exclude/admission.json",
        "admission_request_hash": digest(b"admission"),
        "output_directory": ".local-exclude/coordination/slice/run"
    });
    let path = repository.write("delegated-alpha4.json", &format!("{delegated}\n"));
    let (request, policy) = read_request_with_output_policy(&path).unwrap();
    assert!(matches!(request, DeliveryRequest::Delegated(_)));
    assert!(policy.prepare_output);
    assert!(policy.bind_usage);
}

// delegated delivery는 managed request를 재해석하지 않고 host 전용 schema와 exact
// admission request를 요구하며 alpha2도 같은 closed field set만 확장합니다.
#[test]
fn delegated_delivery_request_has_a_closed_host_shape() {
    let repository = TestRepository::new("review-delegated-delivery-request");
    let valid = serde_json::json!({
        "schema": "yo.slice-review-delegated-delivery-request/v1alpha1",
        "egress_request_path": ".local-exclude/egress.json",
        "egress_request_hash": digest(b"egress"),
        "admission_request_path": ".local-exclude/admission.json",
        "admission_request_hash": digest(b"admission"),
        "output_directory": ".local-exclude/coordination/slice/run"
    });
    let path = repository.write("valid.json", &format!("{valid}\n"));
    assert!(matches!(
        read_request(&path).unwrap(),
        DeliveryRequest::Delegated(_)
    ));

    let mut alpha2 = valid.clone();
    alpha2["schema"] = "yo.slice-review-delegated-delivery-request/v1alpha2".into();
    let path = repository.write("alpha2.json", &format!("{alpha2}\n"));
    let DeliveryRequest::Delegated(alpha2) = read_request(&path).unwrap() else {
        panic!("alpha2 delegated request selected another delivery protocol");
    };
    assert_eq!(
        alpha2.schema,
        "yo.slice-review-delegated-delivery-request/v1alpha2"
    );

    let mut extra = valid;
    extra["provider_request_limit"] = 1.into();
    let path = repository.write("extra.json", &format!("{extra}\n"));
    assert!(read_request(&path).unwrap_err().contains("unknown field"));
}

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

// 로컬 준비 단계가 exact output child를 만들고 쓰기 probe를 회수하므로, 존재하지
// 않는 디렉터리가 Provider request 전의 delivery 실패로 잘못 계산되지 않습니다.
#[test]
fn output_directory_preparation_creates_and_checks_exact_child() {
    let repository = TestRepository::new("review-delivery-output-prepare");
    let coordination = repository.path.join("coordination");
    std::fs::create_dir(&coordination).unwrap();
    let output = coordination.join("attempt-1");

    let prepared = prepare_output_directory_at(&coordination, &output).unwrap();
    assert_eq!(prepared, std::fs::canonicalize(&output).unwrap());
    assert!(std::fs::read_dir(&prepared).unwrap().next().is_none());
    prepare_output_directory_at(&coordination, &output).unwrap();
}

// 이전 attempt 파일이 남은 경로는 준비 단계에서 거부해 새 claim이나 외부 요청이
// 기존 결과와 섞이지 않도록 합니다.
#[test]
fn output_directory_preparation_rejects_nonempty_directory() {
    let repository = TestRepository::new("review-delivery-output-nonempty");
    let coordination = repository.path.join("coordination");
    let output = coordination.join("attempt-1");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("claim.json"), b"claimed").unwrap();

    assert!(
        prepare_output_directory_at(&coordination, &output)
            .unwrap_err()
            .contains("must be empty")
    );
}

#[cfg(unix)]
// Slice 안을 가리키더라도 최종 symlink는 실제 output 소유권을 흐리므로 claim 전에
// 거부해 경로 교체나 alias를 통한 결과 덮어쓰기를 막습니다.
#[test]
fn output_directory_preparation_rejects_final_symlink() {
    let repository = TestRepository::new("review-delivery-output-symlink");
    let coordination = repository.path.join("coordination");
    let target = coordination.join("target");
    let output = coordination.join("attempt-1");
    std::fs::create_dir_all(&target).unwrap();
    symlink(&target, &output).unwrap();

    assert!(
        prepare_output_directory_at(&coordination, &output)
            .unwrap_err()
            .contains("real directory")
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
// continuation launch는 TUI나 model override를 열지 않고 exact Session의 print resume
// argv, 기존 Session repository, immutable delta stdin만 child에 전달합니다.
#[test]
fn continuation_launch_uses_exact_print_resume_arguments() {
    let repository = TestRepository::new("review-continuation-delivery-launch");
    let output = repository.path.join("output");
    let sessions = repository.path.join("sessions");
    std::fs::create_dir(&output).unwrap();
    std::fs::create_dir(&sessions).unwrap();
    let executable = repository.write(
        "yo",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$YO_SESSION_REPOSITORY/argv\"\ncat > \"$YO_SESSION_REPOSITORY/stdin\"\nprintf 'reviewed\\n'\n",
    );
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();
    let mut delivery = authorized();
    delivery.review_kind = "finding_resolution";
    delivery.fresh_session = false;
    delivery.session_id = Some("01890f00-0000-7000-8000-000000000001".to_owned());
    delivery.packet_bytes = b"delta packet".to_vec();

    let capture = execute_continuation_once(
        &executable,
        &repository.path,
        &output,
        &sessions,
        delivery.session_id.as_deref().unwrap(),
        &delivery,
    );

    assert!(capture.status.unwrap().success());
    assert_eq!(capture.stdout, b"reviewed\n");
    assert_eq!(
        std::fs::read_to_string(sessions.join("argv")).unwrap(),
        "-p\n--resume\n01890f00-0000-7000-8000-000000000001\n"
    );
    assert_eq!(
        std::fs::read(sessions.join("stdin")).unwrap(),
        b"delta packet"
    );

    let mut delegated = authorized_host();
    delegated.review_kind = "finding_resolution";
    delegated.fresh_session = false;
    delegated.session_id = delivery.session_id.clone();
    delegated.packet_bytes = b"delegated delta".to_vec();
    let capture = execute_delegated_continuation_once(
        &executable,
        &repository.path,
        &output,
        &sessions,
        delegated.session_id.as_deref().unwrap(),
        &delegated,
        None,
    );
    assert!(capture.status.unwrap().success());
    assert_eq!(
        std::fs::read_to_string(sessions.join("argv")).unwrap(),
        "-p\n--resume\n01890f00-0000-7000-8000-000000000001\n"
    );
    assert_eq!(
        std::fs::read(sessions.join("stdin")).unwrap(),
        b"delegated delta"
    );
}

#[cfg(unix)]
// fresh delegated launch는 managed `--no-tools`를 주장하지 않고 승인된 host와
// read-only profile만 argv로 고정합니다.
#[test]
fn delegated_launch_uses_exact_host_read_only_arguments() {
    let repository = TestRepository::new("review-delegated-delivery-launch");
    let executable = repository.write(
        "yo",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$YO_SESSION_REPOSITORY.argv\"\ncat > \"$YO_SESSION_REPOSITORY.stdin\"\nprintf 'reviewed\\n'\n",
    );
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).unwrap();

    for host in ["codex", "grok"] {
        let output = repository.path.join(format!("output-{host}"));
        std::fs::create_dir(&output).unwrap();
        let mut delivery = authorized_host();
        delivery.host = host.to_owned();
        let capture =
            execute_delegated_once(&executable, &repository.path, &output, &delivery, None);

        assert!(capture.status.unwrap().success());
        assert_eq!(
            std::fs::read_to_string(output.join("sessions.argv")).unwrap(),
            format!("-p\n--model\nhost:{host}\n--sandbox\nread-only\n")
        );
        assert_eq!(
            std::fs::read(output.join("sessions.stdin")).unwrap(),
            b"review packet"
        );
    }
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
