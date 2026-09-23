use super::*;

// 이전의 유효한 Anchor 뒤에 새 Turn 명령만 durable하게 남고 완결 Anchor가 생기지
// 않았다면 재개 경계는 과거 Anchor로 되돌아가지 않고 전체 Session을 실행 불가로 닫는다.
#[test]
fn rejects_an_unanchored_suffix_instead_of_falling_back_to_an_older_anchor() {
    let (mut repository, continuation) = durable_resumable_session();
    let before = repository.entries.lock().unwrap().len();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let next_turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    let request = BackendRequestEvidence::new(
        "codex.app-server/turn-start/v1",
        BackendIdentity::new("codex.app-server/json-rpc-request/v1", "4"),
        BackendIdentity::new(
            "codex.app-server/accepted-request/v1",
            r#"{"jsonRpcId":4,"turnId":"turn-b"}"#,
        ),
    );
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: binding(),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::new("unfinished"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut session = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    let admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            UserInput::new("unfinished"),
        )))
        .unwrap();
    enqueue_submission(&mut session, admission);
    let deadline = Instant::now() + Duration::from_secs(1);
    while repository.entries.lock().unwrap().len() < before + 2 {
        session.poll().unwrap();
        assert!(
            Instant::now() < deadline,
            "the unfinished command did not reach its durable accepted-request suffix"
        );
        thread::sleep(Duration::from_millis(1));
    }
    session.shutdown().unwrap();

    let error = recover_stored_session_continuation(&mut repository, session_id)
        .expect_err("an unfinished durable suffix must invalidate the older Anchor");
    assert!(
        error
            .to_string()
            .contains("no newest durable Continuation Anchor")
    );
}

// backend가 durable Anchor와 다른 identity를 반환하면 snapshot이나 frontend 상태를
// 공개하기 전에 startup을 닫고, cleanup 뒤에도 저장소에는 새 record가 없어야 한다.
#[test]
fn resumed_agent_rejects_identity_drift_before_snapshot_publication() {
    let (repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let before = repository.entries.lock().unwrap().len();
    let mut drifted = binding();
    drifted = BackendBindingEvidence::new(
        drifted.backend_kind(),
        drifted.backend_version(),
        drifted.binding_identity().clone(),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"other","provider":"openai"}"#,
        ),
        drifted.session_locator().clone(),
        drifted.continuation_strategy(),
    );
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: drifted,
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let error = match AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository.clone(),
        || false,
    ) {
        Ok(_) => panic!("identity drift must not create a resumed Session"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("binding identity different"));
    assert_eq!(repository.entries.lock().unwrap().len(), before);
}

// native resume identity가 맞아도 필수 full snapshot을 저장하지 못하면 새 command를
// 받을 Session을 반환하지 않고, 실패한 append가 durable prefix를 바꾸지 않는다.
#[test]
fn resumed_agent_rejects_snapshot_failure_before_command_admission() {
    let (repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let before = repository.entries.lock().unwrap().len();
    repository.fail_append.store(true, Ordering::Release);
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: binding(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let error = match AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository.clone(),
        || false,
    ) {
        Ok(_) => panic!("snapshot failure must not create a resumed Session"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("complete Journal snapshot"));
    assert_eq!(repository.entries.lock().unwrap().len(), before);
}
