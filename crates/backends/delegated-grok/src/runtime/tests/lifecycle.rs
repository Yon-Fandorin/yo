use super::*;

// 새 Session은 ACP v1 초기화 뒤 cached_token만 인증하고 session/new를 호출하며,
// 반환된 Grok Session ID를 backend-managed Continuation 신원으로 보존합니다.
#[test]
fn initializes_with_cached_login_and_opens_a_durable_session() {
    let session_id = session(1);
    let (mut backend, sent) = backend([response(3, json!({ "sessionId": "grok-session-a" }))]);

    let evidence = backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();

    let BackendCommandEvidence::BindingOpened(evidence) = evidence else {
        panic!("session/new must return binding evidence");
    };
    assert_eq!(evidence.backend_kind(), "grok-build-acp");
    assert_eq!(evidence.backend_version(), "grok/1.0.5");
    assert_eq!(evidence.session_locator().value(), "grok-session-a");
    assert_eq!(
        evidence.continuation_strategy(),
        ContinuationStrategy::BackendManagedState
    );

    let sent = sent.0.borrow();
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[0]["params"]["clientCapabilities"], json!({}));
    assert_eq!(sent[1]["method"], "authenticate");
    assert_eq!(sent[1]["params"]["methodId"], "cached_token");
    assert_eq!(sent[2]["method"], "session/new");
    assert_eq!(sent[2]["params"]["cwd"], "/workspace");
}

// 읽기 전용 Grok Session은 일반 ACP payload를 바꾸지 않되 durable binding에 제한
// execution profile을 기록해 이후 resume이 같은 process 정책을 복원할 수 있게 합니다.
#[test]
fn read_only_review_records_a_distinct_durable_binding() {
    let (mut backend, sent) = backend_with_profile(
        [response(3, json!({ "sessionId": "grok-session-a" }))],
        true,
    );

    let evidence = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap();
    let BackendCommandEvidence::BindingOpened(binding) = evidence else {
        panic!("session/new must return the restricted binding");
    };
    assert_eq!(
        binding.binding_identity().schema(),
        "grok.acp/session-binding/v1alpha1"
    );
    assert!(
        binding
            .binding_identity()
            .value()
            .contains("yo.delegated-review-execution/v1alpha1")
    );
    assert_eq!(sent.0.borrow()[2]["method"], "session/new");
}

// Grok이 cached_token을 광고하지 않으면 API key나 브라우저 flow로 자동 전환하지 않고,
// 별도 과금 가능성을 피하기 위해 grok login 안내가 있는 Initialization 실패로 닫습니다.
#[test]
fn refuses_to_fall_back_when_cached_login_is_unavailable() {
    for methods in [&[][..], &["grok.com"][..]] {
        let (peer, sent) = FakePeer::new([initialize_response(1, methods, true)]);
        let mut backend = Backend::new_uninitialized(
            AcpClient::new(peer, Duration::from_secs(1)),
            "/workspace".to_owned(),
            false,
        );

        let failure = backend
            .execute_command(AgentCommand::CreateSession {
                session_id: session(1),
            })
            .unwrap_err();

        assert_eq!(failure.kind(), BackendFailureKind::Initialization);
        assert!(failure.message().contains("grok login"));
        assert_eq!(sent.0.borrow().len(), 1);
    }
}

// cached_token이 광고됐지만 저장 login이 거절되면 원래 인증 실패를 보존하면서도
// 다른 credential flow로 전환하지 않고 grok login 재실행 경로를 명시합니다.
#[test]
fn rejected_cached_login_has_actionable_login_guidance() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, &["cached_token", "grok.com"], true),
        error_response(2, -32000, "token expired"),
    ]);
    let mut backend = Backend::new_uninitialized(
        AcpClient::new(peer, Duration::from_secs(1)),
        "/workspace".to_owned(),
        false,
    );

    let failure = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert!(failure.message().contains("run `grok login`"));
    assert!(failure.message().contains("token expired"));
    assert_eq!(sent.0.borrow().len(), 2);
}

// 첫 agent_message_chunk가 prompt 수락 증거가 되고, 같은 message의 text delta와 종료가
// 하나의 AgentMessage Activity 및 재개 가능한 완료 Turn으로 순서대로 노출됩니다.

#[cfg(unix)]
// initialize 전에 종료하는 실제 process의 stderr는 안전한 고정 진단만 cleanup과 합성한다.
#[test]
fn early_process_failure_reports_safe_sandbox_diagnostics_without_stderr_secrets() {
    use std::{fs, os::unix::fs::PermissionsExt};

    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    for failure_kind in ["unknown", "bwrap", "profile"] {
        let directory = std::env::temp_dir().join(format!(
            "yo-grok-startup-diagnostic-{}-{}",
            std::process::id(),
            yo_core::WorkspaceHostId::new().unwrap(),
        ));
        fs::create_dir(&directory).unwrap();
        let fixture = Fixture(directory);
        let script = fixture.0.join("fake-grok");
        let stderr = if failure_kind == "bwrap" {
            format!(
                "{}\ncould not create bwrap placeholder for read-deny path /private/secret-canary; refusing partial sandbox\nAuthorization: Bearer credential-canary",
                "x".repeat(20000)
            )
        } else if failure_kind == "profile" {
            "error: could not apply the 'secret-canary' sandbox profile; see the warning above for the cause. Refusing to start with its protections missing.\nAuthorization: Bearer credential-canary".to_owned()
        } else {
            "Authorization: Bearer credential-canary\nunknown secret-canary startup error"
                .to_owned()
        };
        let stderr = stderr.replace('\'', "'\\''");
        fs::write(&script, format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > arguments\nprintf '%s\\n' '{stderr}' >&2\nexit 1\n"
        )).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let config = GrokBackendConfig::new(&fixture.0)
            .with_executable(script.as_os_str())
            .with_read_only_review(true)
            .with_request_timeout(Duration::from_secs(2))
            .with_shutdown_timeout(Duration::from_secs(1));

        let failure = GrokBackend::verify(config).unwrap_err();
        let message = failure.message();
        assert!(!message.contains("credential-canary"));
        assert!(!message.contains("secret-canary"));
        assert!(message.len() < 1500, "diagnostic was not bounded");
        if failure_kind == "bwrap" {
            assert!(message.contains("bubblewrap placeholder"), "{message}");
            assert!(message.contains("requested sandbox remains required"));
        } else if failure_kind == "profile" {
            assert!(
                message.contains("could not apply requested sandbox protections"),
                "{message}"
            );
            assert!(message.contains("check host sandbox configuration"));
            assert!(!message.contains("bubblewrap"));
        } else {
            assert!(
                message.contains("withheld to protect credentials"),
                "{message}"
            );
        }
        let arguments = fs::read_to_string(fixture.0.join("arguments")).unwrap();
        assert!(arguments.starts_with("--sandbox\nread-only\n"));
        assert!(!arguments.lines().any(|argument| argument == "off"));
    }
}
