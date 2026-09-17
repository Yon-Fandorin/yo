use super::*;

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
    fs::create_dir(&output).unwrap();
    fs::write(output.join(".review.stdout.tmp"), b"occupied").unwrap();

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
