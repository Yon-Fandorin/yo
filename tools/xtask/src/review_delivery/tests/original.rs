use super::*;

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
