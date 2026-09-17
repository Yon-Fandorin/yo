use super::*;

// idle Session의 worker 안에서 교체를 연속 commit해도 다음 Turn 없이 새 binding과 이전
// durable Anchor의 exact replay 조합으로 즉시 다시 열 수 있어야 한다.
#[test]
fn idle_replacement_is_immediately_resumable_without_another_turn() {
    let (mut repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let replacement = replacement_binding();
    let second_replacement = second_replacement_binding();
    let current = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target.clone()),
            evidence: target.binding().clone(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let candidate = ScriptedBackend::new([
        BackendScriptStep::ReplaceBinding {
            target: Box::new(target.clone()),
            evidence: replacement.clone(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let second_target = BackendResumeTarget::new(
        session_id,
        2,
        replacement.clone(),
        target
            .source()
            .expect("stored continuation has a source")
            .sequence(),
    )
    .with_model_replay(target.model_replay().clone())
    .with_input_image_history(target.input_image_history())
    .with_context_state(
        target.context_policy().cloned(),
        target.context_epoch(),
        target.model_replay_groups().to_vec(),
    )
    // 첫 replacement epoch는 아직 request를 받지 않았고 다음 delta에 새 contract가 필요하다.
    .with_replay_contract_rebind_required(true)
    .with_binding_has_accepted_request(false);
    let second_candidate = ScriptedBackend::new([
        BackendScriptStep::ReplaceBinding {
            target: Box::new(second_target),
            evidence: second_replacement.clone(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut session = AgentSession::start_cancellable_with_continuation(
        current,
        continuation,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();

    let outcome = session
        .replace_backend(Box::new(candidate), || false)
        .unwrap();
    assert!(outcome.cleanup_failure().is_none());
    let outcome = session
        .replace_backend(Box::new(second_candidate), || false)
        .unwrap();
    assert!(outcome.cleanup_failure().is_none());
    session.shutdown().unwrap();

    let recovered = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(recovered.target().epoch(), 3);
    assert_eq!(recovered.target().binding(), &second_replacement);
    assert!(recovered.target().replay_contract_rebind_required());
    assert!(!recovered.target().binding_has_accepted_request());
}

// replacement transition의 durable append가 실패하면 candidate만 정리하고 기존 backend가
// 같은 Session의 다음 command를 실제로 받아 처리해야 하며 partial epoch는 저장하지 않는다.
#[test]
fn failed_idle_replacement_keeps_the_previous_backend_usable() {
    let (repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    let current = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target.clone()),
            evidence: target.binding().clone(),
        },
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn,
            input: UserInput::new("still usable"),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn,
            outcome: crate::TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let candidate = ScriptedBackend::new([
        BackendScriptStep::ReplaceBinding {
            target: Box::new(target),
            evidence: replacement_binding(),
        },
        BackendScriptStep::Shutdown(Err(crate::BackendFailure::new(
            crate::BackendFailureKind::Cleanup,
            "candidate cleanup failed",
        ))),
    ]);
    let mut session = AgentSession::start_cancellable_with_continuation(
        current,
        continuation,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    let entries_before = repository.entries.lock().unwrap().len();
    repository.fail_append.store(true, Ordering::Release);

    let error = session
        .replace_backend(Box::new(candidate), || false)
        .unwrap_err();
    assert!(matches!(
        error,
        crate::AgentSessionError::Multiple {
            primary,
            additional,
        } if matches!(*primary, crate::AgentSessionError::Runtime(_))
            && matches!(*additional, crate::AgentSessionError::BackendCleanup(_))
    ));
    assert_eq!(repository.entries.lock().unwrap().len(), entries_before);

    let mut admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            UserInput::new("still usable"),
        )))
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        admission = session.retry(pending).unwrap();
    }
    let transcript = session.transcript_reader();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        session.poll().unwrap();
        if transcript.read_after(None).entries().iter().any(|entry| {
            matches!(
                entry.record(),
                crate::TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn: finished,
                    ..
                }) if *finished == turn
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the previous backend did not finish work after a rejected replacement"
        );
        thread::sleep(Duration::from_millis(1));
    }
    session.shutdown().unwrap();
}

// exact-replay replacement은 기존 Anchor의 전체 snapshot을 먼저 게시한 뒤 이전 epoch를
// 닫고 새 binding epoch를 열며, replay와 Session identity는 그대로 이어지는지 검증합니다.
#[test]
fn replacement_resume_publishes_a_new_binding_epoch_from_the_exact_anchor() {
    let (mut repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let before = repository.entries.lock().unwrap().len();
    let previous_replay = target.model_replay().clone();
    let session_id = continuation.descriptor().session_id();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    let replacement = replacement_binding();
    let backend = ScriptedBackend::new([
        BackendScriptStep::ReplaceBinding {
            target: Box::new(target),
            evidence: replacement.clone(),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: UserInput::new("continue on replacement"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "replacement/request/v1",
                BackendIdentity::new("replacement/exchange/v1", "request-2"),
                BackendIdentity::new("replacement/request/v1", "request-2"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "replacement/outcome/v1",
                "outcome-2",
            ))
            .with_replay(ModelReplayDelta::new(
                Some(ModelReplayContract::new("replacement system", Vec::new())),
                vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: "continue on replacement".to_owned(),
                        refusal: None,
                    },
                    ModelReplayItem::Message {
                        role: ModelReplayRole::Assistant,
                        content: "replacement complete".to_owned(),
                        refusal: None,
                    },
                ],
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let mut session = AgentSession::start_cancellable_with_replacement_continuation(
        backend,
        continuation,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    let mut admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            UserInput::new("continue on replacement"),
        )))
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        admission = session.retry(pending).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while repository.entries.lock().unwrap().len() < before + 4 {
        session.poll().unwrap();
        assert!(
            Instant::now() < deadline,
            "the replacement Turn did not publish its durable Anchor"
        );
        thread::sleep(Duration::from_millis(1));
    }
    session.shutdown().unwrap();

    assert_eq!(repository.entries.lock().unwrap().len(), before + 4);
    let recovered = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(recovered.target().epoch(), 2);
    assert_eq!(recovered.target().binding(), &replacement);
    assert!(!recovered.target().replay_contract_rebind_required());
    assert_eq!(
        recovered.target().model_replay().items().len(),
        previous_replay.items().len() + 2
    );
}

// nonempty inherited archive는 replay가 text-only여도 완전한 입력 부재를 증명하지 못하므로
// Unknown으로 복구합니다. ExactReplay의 실행 가능한 replay는 이 보수적 archive evidence만으로
// text-only admission을 막지 않고, 실제 image input은 별도 capability 경계를 계속 적용합니다.
#[test]
fn prepared_exact_fork_starts_and_publishes_one_atomic_child_snapshot() {
    use crate::{
        BackendResumeSource, JournalSequence, fixture_descriptor,
        journal::codec::{decode, recover},
    };
    let (parent_repository, parent) = durable_resumable_session();
    assert_eq!(
        parent.target().input_image_history(),
        InputImageHistory::TextOnly
    );
    let before = parent_repository
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.record().payload().to_owned())
        .collect::<Vec<_>>();
    let child_id = SessionId::new().unwrap();
    let candidate = exact_child_binding();
    let child = parent
        .prepare_exact_fork(fixture_descriptor(child_id), candidate.clone())
        .unwrap();
    assert_eq!(child.next_turn_id(), 1);
    assert!(child.submission_ids().is_empty());
    assert_eq!(
        child.target().source(),
        Some(BackendResumeSource::InitialFork(JournalSequence::new(2)))
    );
    assert_eq!(child.target().epoch(), 1);
    assert_eq!(child.target().context_epoch(), Some(1));
    assert_eq!(
        child.target().input_image_history(),
        InputImageHistory::Unknown
    );
    assert_eq!(
        child.target().model_replay(),
        parent.target().model_replay()
    );
    let target = child.target().clone();
    let repository = MemoryRepository::default();
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: candidate,
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut session = AgentSession::start_cancellable_with_continuation(
        backend,
        child,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    let entries = repository.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].record().kind(), DurableRecordKind::Snapshot);
    let snapshot = decode(entries[0].record().payload()).unwrap();
    let recovered = recover(&[snapshot]).unwrap();
    assert_eq!(recovered.descriptor().unwrap().session_id(), child_id);
    assert_eq!(recovered.initial_fork_seed(), Some(JournalSequence::new(2)));
    assert_eq!(recovered.binding_epoch(), Some(1));
    assert_eq!(recovered.model_replay(), parent.target().model_replay());
    assert!(recovered.records().iter().all(|entry| !matches!(
        entry.record(),
        JournalRecord::CommandCommitted(_) | JournalRecord::BackendRequestAccepted(_)
    )));
    let reopened = build_continuation(recovered, child_id).unwrap();
    assert_eq!(
        reopened.target().model_replay(),
        parent.target().model_replay()
    );
    assert_eq!(
        reopened.target().input_image_history(),
        InputImageHistory::Unknown
    );
    assert!(reopened.inherited_history().is_some_and(|history| {
        history
            .sections()
            .iter()
            .any(|section| !section.records().is_empty())
    }));
    assert_eq!(reopened.next_turn_id(), 1);
    session.shutdown().unwrap();
    let after = parent_repository
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.record().payload().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(after, before);
}

// 준비된 child의 최초 snapshot append가 실패하면 실행 가능한 child를 반환하지 않고 candidate
// backend를 정리하며 부모의 durable bytes를 그대로 유지합니다.
#[test]
fn prepared_exact_fork_append_failure_never_publishes_executable_child() {
    use crate::fixture_descriptor;
    let (parent_repository, parent) = durable_resumable_session();
    let before = parent_repository
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.record().payload().to_owned())
        .collect::<Vec<_>>();
    let candidate = exact_child_binding();
    let child = parent
        .prepare_exact_fork(
            fixture_descriptor(SessionId::new().unwrap()),
            candidate.clone(),
        )
        .unwrap();
    let repository = MemoryRepository::default();
    repository.fail_append.store(true, Ordering::Release);
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(child.target().clone()),
            evidence: candidate,
        },
        BackendScriptStep::Shutdown(Err(crate::BackendFailure::new(
            crate::BackendFailureKind::Cleanup,
            "fork candidate cleanup observed",
        ))),
    ]);
    let result = AgentSession::start_cancellable_with_continuation(
        backend,
        child,
        repository.clone(),
        || false,
    );
    let error = match result {
        Ok(_) => panic!("failed bootstrap must not return a child"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("complete Journal snapshot"));
    assert!(
        error
            .to_string()
            .contains("fork candidate cleanup observed")
    );
    assert!(repository.entries.lock().unwrap().is_empty());
    let after = parent_repository
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.record().payload().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(after, before);
}
