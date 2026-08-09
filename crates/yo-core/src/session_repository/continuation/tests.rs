use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::*;
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, AgentIntent, AgentRuntime, AgentSession, AgentSessionPoll, BackendBindingEvidence,
    BackendCommandEvidence, BackendEvent, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendScriptStep, CommandAdmission, ContinuationStrategy,
    InputSubmission, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ReplayExecutor, RuntimePoll, ScriptedBackend, SubmissionId, TurnId, TurnRef, UserInput,
    journal::SessionJournal,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, DurableRecordKind, RepositoryEntry,
        RepositoryError, RepositorySequence, SessionRepository,
    },
};

#[derive(Clone, Debug, Default)]
struct MemoryRepository {
    entries: Arc<Mutex<Vec<RepositoryEntry>>>,
    fail_append: Arc<AtomicBool>,
}

impl SessionRepository for MemoryRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        if self.fail_append.swap(false, Ordering::AcqRel) {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "injected append failure".to_owned(),
            }));
        }
        let mut entries = self.entries.lock().unwrap();
        let sequence = RepositorySequence::new(u64::try_from(entries.len()).unwrap() + 1);
        entries.push(RepositoryEntry::new(sequence, record));
        Ok(AppendReceipt::new(sequence))
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

impl SessionWriterRepository for MemoryRepository {
    fn acquire_session_writer(&mut self, _session_id: SessionId) -> Result<(), RepositoryError> {
        Ok(())
    }
}

fn binding() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "codex-app-server",
        "codex_cli_rs/0.146.0",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            r#"{"sessionId":"thread-a","threadId":"thread-a"}"#,
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"gpt-test","provider":"openai"}"#,
        ),
        BackendIdentity::new("codex.app-server/thread-locator/v1", "thread-a"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
        },
    )
}

fn replacement_binding() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        binding().backend_kind(),
        "replacement/v1",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            r#"{"sessionId":"thread-b","threadId":"thread-b"}"#,
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"replacement-model","provider":"openai"}"#,
        ),
        binding().session_locator().clone(),
        binding().continuation_strategy(),
    )
}

fn second_replacement_binding() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        binding().backend_kind(),
        "replacement/v2",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            r#"{"sessionId":"thread-c","threadId":"thread-c"}"#,
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"second-replacement-model","provider":"openai"}"#,
        ),
        binding().session_locator().clone(),
        binding().continuation_strategy(),
    )
}

fn stored_submission() -> SubmissionId {
    SubmissionId::from_uuid(uuid::Builder::from_random_bytes([9; 16]).into_uuid()).unwrap()
}

fn durable_resumable_session() -> (MemoryRepository, StoredSessionContinuation) {
    let session_id = crate::fixture_session(1);
    let turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(1).unwrap()),
    );
    let submission = stored_submission();
    let activity = ActivityRef::new(turn, ActivityId::new(std::num::NonZeroU64::new(1).unwrap()));
    let request = BackendRequestEvidence::new(
        "codex.app-server/turn-start/v1",
        BackendIdentity::new("codex.app-server/json-rpc-request/v1", "3"),
        BackendIdentity::new(
            "codex.app-server/accepted-request/v1",
            r#"{"jsonRpcId":3,"turnId":"turn-a"}"#,
        ),
    );
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(binding()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: UserInput::new("resume me"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request),
        },
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta("durable streamed answer".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "codex.app-server/turn-outcome/v1",
                "turn-a",
            ))
            .with_replay(ModelReplayDelta::new(
                Some(ModelReplayContract::new("system", Vec::new())),
                vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: "resume me".to_owned(),
                        refusal: None,
                    },
                    ModelReplayItem::Message {
                        role: ModelReplayRole::Assistant,
                        content: "durable streamed answer".to_owned(),
                        refusal: None,
                    },
                ],
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut repository = MemoryRepository::default();
    let journal = SessionJournal::with_repository_and_descriptor(
        Box::new(repository.clone()),
        crate::fixture_descriptor(session_id),
    );
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime.initialize_durability();
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn,
                input: UserInput::new("resume me"),
            },
            submission,
        )
        .unwrap();
    loop {
        match runtime.poll_event().unwrap() {
            RuntimePoll::Event(crate::AgentEvent::TurnFinished {
                turn: finished,
                outcome: crate::TurnOutcome::Completed,
            }) if finished == turn => break,
            RuntimePoll::Event(_) => {},
            RuntimePoll::Pending | RuntimePoll::Closed => {
                panic!("the fixture backend ended before publishing its resumable Turn")
            },
        }
    }
    runtime.shutdown().unwrap();
    let continuation = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    (repository, continuation)
}

// 완결된 Turn의 최신 Anchor는 해당 epoch의 versioned Codex locator와 결합되고,
// 다음 Turn ID와 기존 SubmissionId까지 한 번의 검증된 복구 결과에서 이어진다.
#[test]
fn derives_one_executable_plan_from_the_newest_durable_anchor() {
    let (_repository, continuation) = durable_resumable_session();

    assert_eq!(continuation.target().epoch(), 1);
    assert_eq!(
        continuation.target().binding().session_locator().value(),
        "thread-a"
    );
    assert_eq!(continuation.next_turn_id(), 2);
    assert_eq!(continuation.submission_ids().len(), 1);
    assert_eq!(
        continuation.target().model_replay().contract(),
        Some(&ModelReplayContract::new("system", Vec::new()))
    );
    assert_eq!(continuation.target().model_replay().items().len(), 2);
}

// 이전의 유효한 Anchor 뒤에 새 Turn 명령만 durable하게 남고 완결 Anchor가 생기지
// 않았다면 재개 경계는 과거 Anchor로 되돌아가지 않고 전체 Session을 실행 불가로 닫는다.
#[test]
fn rejects_an_unanchored_suffix_instead_of_falling_back_to_an_older_anchor() {
    let (mut repository, continuation) = durable_resumable_session();
    let before = repository.entries.lock().unwrap().len();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let next_turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
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
    let mut admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            UserInput::new("unfinished"),
        )))
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        admission = session.retry(pending).unwrap();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while repository.entries.lock().unwrap().len() < before + 2 {
        session.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "the unfinished command did not reach its durable accepted-request suffix"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
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
        target.source_anchor_sequence(),
    )
    .with_model_replay(target.model_replay().clone());
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
}

// replacement transition의 durable append가 실패하면 candidate만 정리하고 기존 backend가
// 같은 Session의 다음 command를 실제로 받아 처리해야 하며 partial epoch는 저장하지 않는다.
#[test]
fn failed_idle_replacement_keeps_the_previous_backend_usable() {
    let (repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
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
            std::time::Instant::now() < deadline,
            "the previous backend did not finish work after a rejected replacement"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
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
    let turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
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
                None,
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while repository.entries.lock().unwrap().len() < before + 4 {
        session.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "the replacement Turn did not publish its durable Anchor"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    session.shutdown().unwrap();

    assert_eq!(repository.entries.lock().unwrap().len(), before + 4);
    let recovered = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(recovered.target().epoch(), 2);
    assert_eq!(recovered.target().binding(), &replacement);
    assert_eq!(
        recovered.target().model_replay().items().len(),
        previous_replay.items().len() + 2
    );
}

// 저장 text delta가 semantic sequence에는 구멍을 남겨도 재개 후 첫 새 Turn은 마지막
// durable cutoff 다음 번호에서 시작하고, 기존 SubmissionId 재사용도 backend 전에 막는다.
#[test]
fn resumed_agent_continues_sequences_and_admission_identities_after_streamed_text() {
    let (mut repository, continuation) = durable_resumable_session();
    let target = continuation.target().clone();
    let session_id = continuation.descriptor().session_id();
    let second_turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
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
    let startup_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match session.poll().unwrap() {
            AgentSessionPoll::Changed => break,
            AgentSessionPoll::Pending => {
                assert!(
                    std::time::Instant::now() < startup_deadline,
                    "the resumed Session did not publish its startup snapshot"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
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
    let mut admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            second_submission,
            UserInput::new("continue"),
        )))
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        std::thread::sleep(std::time::Duration::from_millis(1));
        admission = session.retry(pending).unwrap();
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while repository.entries.lock().unwrap().len() < before + 3 {
        if let Err(error) = session.poll() {
            panic!("the resumed Turn failed before durable completion: {error}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the resumed Turn did not publish its snapshot, request, and Anchor"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    session.shutdown().unwrap();
    let recovered = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(recovered.next_turn_id(), 3);
    assert_eq!(recovered.submission_ids().len(), 2);
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
