use super::*;

// payload 없는 비밀 제출 영수증도 재개·fork가 이전 Anchor로 돌아가지 못하게 영구 차단한다.
#[test]
fn protected_input_receipt_permanently_blocks_resume_and_fork_recovery() {
    let (mut repository, continuation) = durable_resumable_session();
    let session_id = continuation.descriptor().session_id();
    let request = crate::ActivityRequestRef::new(
        ActivityRef::new(
            TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap())),
            ActivityId::new(NonZeroU64::new(1).unwrap()),
        ),
        crate::RequestId::new(NonZeroU64::new(1).unwrap()),
    );
    let mut journal = SessionJournal::with_repository_and_continuation(
        Box::new(repository.clone()),
        &continuation,
    );
    journal.initialize_durability();
    assert!(journal.append_committed_command_transactionally(
        AgentCommand::RespondToActivity {
            request,
            response: crate::ActivityResponse::SecretInputSubmitted,
        },
        &[],
    ));

    let error = recover_stored_session_continuation(&mut repository, session_id)
        .expect_err("a protected-input receipt must make the Session terminal");
    assert!(
        error.to_string().contains("cannot be resumed or forked"),
        "{error:?}"
    );
}

// BackendManagedState의 image-bearing command를 durable하게 복구한 뒤에는 다음 text Turn도
// 보존된 ContainsImages evidence를 다시 확인하므로, capability가 Unknown이면 provider에
// 보내지지 않고 correlated rejection으로 끝나야 합니다.
#[test]
fn recovers_image_history_and_guards_a_later_text_turn() {
    let (_repository, continuation) = durable_resumable_image_session();
    assert_eq!(
        continuation.target().input_image_history(),
        InputImageHistory::ContainsImages
    );
    let target = continuation.target().clone();
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: managed_binding(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut session = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        MemoryRepository::default(),
        || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        session
            .dispatch(AgentIntent::submit("text after the image").unwrap())
            .unwrap(),
        CommandAdmission::Queued
    );
    let SubmissionOutcome::Rejected { rejection, .. } = wait_for_submission_outcome(&mut session)
    else {
        panic!("a text turn after retained image history must be rejected");
    };
    assert_eq!(
        rejection.kind(),
        SubmissionRejectionKind::ImageCapabilityUnknown
    );
    session.shutdown().unwrap();
}

// native resume 성공 뒤에는 이전 명령을 backend로 재전송하지 않고 같은 transcript를
// 먼저 노출하며, 저장소의 첫 후속 기록은 incremental이 아닌 완전 Snapshot이어야 한다.
#[test]
fn resumed_agent_publishes_a_snapshot_before_admitting_new_work() {
    let (repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let before = repository.entries.lock().unwrap().len();
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: binding(),
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

    assert!(
        !session
            .transcript_reader()
            .read_after(None)
            .entries()
            .is_empty()
    );
    session.shutdown().unwrap();
    let entries = repository.entries.lock().unwrap();
    assert_eq!(entries.len(), before + 1);
    assert_eq!(
        entries.last().unwrap().record().kind(),
        DurableRecordKind::Snapshot
    );
}

// 저장 text delta가 semantic sequence에는 구멍을 남겨도 재개 후 첫 새 Turn은 마지막
// durable cutoff 다음 번호에서 시작하고, 기존 SubmissionId 재사용도 backend 전에 막는다.
#[test]
fn resumed_agent_continues_sequences_and_admission_identities_after_streamed_text() {
    let (mut repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let second_turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    let second_submission = SubmissionId::new().unwrap();
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
                turn: second_turn,
                input: UserInput::new("continue"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: second_turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "codex.app-server/turn-outcome/v1",
                "turn-b",
            ))
            .with_replay(ModelReplayDelta::new(
                None,
                vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: "continue".to_owned(),
                        refusal: None,
                    },
                    ModelReplayItem::Message {
                        role: ModelReplayRole::Assistant,
                        content: "continued".to_owned(),
                        refusal: None,
                    },
                ],
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let before = repository.entries.lock().unwrap().len();
    let mut session = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    let startup_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match session.poll().unwrap() {
            AgentSessionPoll::Changed => break,
            AgentSessionPoll::Pending => {
                assert!(
                    Instant::now() < startup_deadline,
                    "the resumed Session did not publish its startup snapshot"
                );
                thread::sleep(Duration::from_millis(1));
            },
            AgentSessionPoll::Closed => panic!("the resumed Session closed during startup"),
        }
    }

    let duplicate = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            stored_submission(),
            UserInput::new("duplicate"),
        )))
        .unwrap_err();
    assert!(matches!(
        duplicate,
        crate::AgentSessionError::DuplicateSubmissionId(id) if id == stored_submission()
    ));
    let admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            second_submission,
            UserInput::new("continue"),
        )))
        .unwrap();
    enqueue_submission(&mut session, admission);

    let deadline = Instant::now() + Duration::from_secs(1);
    while repository.entries.lock().unwrap().len() < before + 3 {
        if let Err(error) = session.poll() {
            panic!("the resumed Turn failed before durable completion: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "the resumed Turn did not publish its snapshot, request, and Anchor"
        );
        thread::sleep(Duration::from_millis(1));
    }
    session.shutdown().unwrap();
    let recovered = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(recovered.next_turn_id(), 3);
    assert_eq!(recovered.submission_ids().len(), 2);
}
