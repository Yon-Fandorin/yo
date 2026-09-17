use super::*;

// continuation은 직전 receipt의 물리적 isolation과 새 admission 결과가 exact하게
// 같을 때만 진행해 같은 reviewer Session의 보안 경계가 중간에 바뀌지 않게 합니다.
#[test]
fn delegated_continuation_pins_prior_execution_isolation() {
    let mut authorized = authorized_host();
    authorized.host = "grok".to_owned();
    authorized.prior_execution_isolation =
        Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE.to_owned());

    require_continuation_isolation(
        &authorized,
        Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
    )
    .unwrap();
    assert!(
        require_continuation_isolation(
            &authorized,
            Some(grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE),
        )
        .unwrap_err()
        .contains("exact prior physical isolation")
    );

    authorized.prior_execution_isolation = None;
    assert!(
        require_continuation_isolation(
            &authorized,
            Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE),
        )
        .is_err()
    );
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
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).unwrap();

    for host in ["codex", "grok"] {
        let output = repository.path.join(format!("output-{host}"));
        fs::create_dir(&output).unwrap();
        let mut delivery = authorized_host();
        delivery.host = host.to_owned();
        let capture =
            execute_delegated_once(&executable, &repository.path, &output, &delivery, None);

        assert!(capture.status.unwrap().success());
        assert_eq!(
            fs::read_to_string(output.join("sessions.argv")).unwrap(),
            format!("-p\n--model\nhost:{host}\n--sandbox\nread-only\n")
        );
        assert_eq!(
            fs::read(output.join("sessions.stdin")).unwrap(),
            b"review packet"
        );
    }
}
