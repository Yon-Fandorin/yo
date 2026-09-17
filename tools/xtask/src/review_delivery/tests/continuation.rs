use super::*;

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

#[cfg(unix)]
// continuation launch는 TUI나 model override를 열지 않고 exact Session의 print resume
// argv, 기존 Session repository, immutable delta stdin만 child에 전달합니다.
#[test]
fn continuation_launch_uses_exact_print_resume_arguments() {
    let repository = TestRepository::new("review-continuation-delivery-launch");
    let output = repository.path.join("output");
    let sessions = repository.path.join("sessions");
    fs::create_dir(&output).unwrap();
    fs::create_dir(&sessions).unwrap();
    let executable = repository.write(
        "yo",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$YO_SESSION_REPOSITORY/argv\"\ncat > \"$YO_SESSION_REPOSITORY/stdin\"\nprintf 'reviewed\\n'\n",
    );
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).unwrap();
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
        fs::read_to_string(sessions.join("argv")).unwrap(),
        "-p\n--resume\n01890f00-0000-7000-8000-000000000001\n"
    );
    assert_eq!(fs::read(sessions.join("stdin")).unwrap(), b"delta packet");

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
        fs::read_to_string(sessions.join("argv")).unwrap(),
        "-p\n--resume\n01890f00-0000-7000-8000-000000000001\n"
    );
    assert_eq!(
        fs::read(sessions.join("stdin")).unwrap(),
        b"delegated delta"
    );
}
