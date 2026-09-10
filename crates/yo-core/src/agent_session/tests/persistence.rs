use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    super::{AgentIntent, AgentSession, CommandAdmission},
    support::{activity, session, turn},
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentCommand, BackendEvent, BackendScriptStep,
    InputReference, InputSubmission, ScriptedBackend, TurnOutcome, UserInput, WorkspaceReference,
    WorkspaceReferenceKind,
    journal::codec::JournalRecord,
    session_repository::{LocalSessionRepository, SessionRepository, journal::JournalRepository},
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-agent-session-persistence-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// 실제 AgentSession worker는 semantic record보다 먼저 cutoff 없는 descriptor envelope를
// 저장해야 한다. frontend가 정상적인 backpressure를 재시도한 뒤 command, streaming
// delta, replacement snapshot, 종료를 처리하고 local JSONL을 다시 열어도 같은
// descriptor, SubmissionId가 붙은 파일·스킬 입력과 검증한 지침, 최종 message revision과 Turn 완료가
// 함께 복구되는지 검증한다.
#[test]
fn live_worker_persists_a_recoverable_session_journal() {
    let directory = TestDirectory::new();
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let descriptor = crate::fixture_descriptor(session());
    let first_turn = turn(1);
    let answer = activity(first_turn, 1);
    let submission_id = "10000000-0000-4000-8000-000000000021"
        .parse()
        .expect("the test submission fixture is a UUIDv4");
    let input = UserInput::with_references(
        "inspect @src/lib.rs",
        vec![InputReference::workspace(
            8..19,
            WorkspaceReference::new(
                "workspace:src/lib.rs",
                "host:one",
                "workspace:one",
                "root:one",
                "src/lib.rs",
                WorkspaceReferenceKind::File,
            )
            .unwrap(),
        )],
    )
    .unwrap();
    use crate::{ResolvedSkill, SkillReference, SkillReferenceScope};
    let selected = SkillReference::new(
        "skill:review",
        "host:one",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "sha256:original",
    );
    let mut references = input.references().to_vec();
    let start = input.as_str().len() + " with ".len();
    references.push(InputReference::skill(
        start..start + "$review".len(),
        selected.clone(),
    ));
    let draft =
        UserInput::with_references(format!("{} with $review", input.as_str()), references).unwrap();
    let snapshot =
        ResolvedSkill::new(selected, "# Review\nInspect the original implementation.").unwrap();
    let input = draft.clone().with_resolved_skill(snapshot.clone()).unwrap();
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first_turn,
            input: input.clone(),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: answer,
            kind: ActivityKind::AgentMessage,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: answer,
            update: ActivityUpdate::TextDelta("draft".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: answer,
            update: ActivityUpdate::TextSnapshot("final".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: answer,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: first_turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = AgentSession::start_cancellable_with_repository(
        backend,
        descriptor.clone(),
        repository,
        || false,
    )
    .unwrap()
    .unwrap();

    use crate::{InputAdmissionHost, SubmissionRejection, SubmissionRejectionKind};

    struct ExactInputHost(UserInput, ResolvedSkill);
    impl InputAdmissionHost for ExactInputHost {
        fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
            if input == &self.0 {
                Ok(())
            } else {
                Err(SubmissionRejection::new(
                    SubmissionRejectionKind::InvalidReference,
                    "fixture input changed",
                ))
            }
        }
        fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
            self.validate(input)?;
            Ok(Some(self.1.clone()))
        }
    }
    app.configure_input_admission(Box::new(ExactInputHost(draft.clone(), snapshot)))
        .unwrap();

    let mut admission = app
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            submission_id,
            draft.clone(),
        )))
        .unwrap();
    let admission_deadline = Instant::now() + Duration::from_secs(1);
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(
            Instant::now() < admission_deadline,
            "the persistence test could not admit its Turn"
        );
        thread::sleep(Duration::from_millis(1));
        admission = app.retry(pending).unwrap();
    }
    app.wait_until_processed(1);
    app.wait_until_no_active_turn();
    app.shutdown().unwrap();

    let repository =
        LocalSessionRepository::open(&directory.0, 1024 * 1024).expect("repository reopens");
    let physical = repository.read_after(session(), None, 16).unwrap();
    assert!(physical.len() >= 2);
    assert_eq!(physical[0].record().journal_cutoff(), None);
    let first = crate::journal::codec::decode(physical[0].record().payload()).unwrap();
    assert!(matches!(
        first.records()[0].record(),
        JournalRecord::SessionDescriptor(observed) if observed == &descriptor
    ));
    drop(repository);
    drop(app);

    let repository =
        LocalSessionRepository::open(&directory.0, 1024 * 1024).expect("repository reopens");
    let recovered = JournalRepository::new(repository)
        .recover(session())
        .unwrap();
    assert_eq!(recovered.descriptor(), Some(&descriptor));
    assert!(recovered.recovery_commit().is_none());
    let committed = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            JournalRecord::CommandCommitted(committed)
                if matches!(committed.command(), AgentCommand::StartTurn { .. }) =>
            {
                Some(committed)
            },
            _ => None,
        })
        .expect("the accepted structured submission is durable");
    assert_eq!(committed.submission_id(), Some(submission_id));
    assert!(matches!(
        committed.command(),
        AgentCommand::StartTurn { input: observed, .. } if observed == &input
    ));
    let terminal = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            JournalRecord::MessageEnded(terminal) => Some(terminal),
            _ => None,
        })
        .expect("the agent message has a durable terminal seal");
    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.final_segment().unwrap().text(), "final");
    assert!(matches!(
        recovered.records().last().unwrap().record(),
        JournalRecord::EventCommitted(crate::AgentEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        })
    ));
}

struct UnexpectedForkReader;

// bounded capture를 구현하지 않은 reader는 기존 unbounded read_session으로 우회하지 않는다.
#[test]
fn historical_fork_reader_defaults_to_unavailable() {
    use crate::session_repository::{SessionForkLimits, StoredSessionReader};
    assert!(
        UnexpectedForkReader
            .read_session_bounded(session(), SessionForkLimits::default())
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
}

impl crate::session_repository::StoredSessionReader for UnexpectedForkReader {
    fn discover(
        &self,
    ) -> Result<
        Vec<crate::session_repository::StoredSession>,
        crate::session_repository::RepositoryError,
    > {
        panic!("fork must reject before storage read")
    }
    fn read_session(
        &self,
        _: crate::SessionId,
    ) -> Result<
        crate::session_repository::StoredSessionSnapshot,
        crate::session_repository::RepositoryError,
    > {
        panic!("fork must reject before storage read")
    }
    fn read_after(
        &self,
        _: crate::SessionId,
        _: Option<crate::session_repository::RepositorySequence>,
        _: usize,
    ) -> Result<
        Vec<crate::session_repository::RepositoryEntry>,
        crate::session_repository::RepositoryError,
    > {
        panic!("fork must reject before storage read")
    }
}

// memory-only와 아직 worker가 처리하지 않은 예약 입력은 저장소에 접근하기 전에 fork 캡처를
// 거부합니다. TUI의 idle 표시만으로 실제 live 상태를 대신하지 않습니다.
#[test]
fn fork_capture_rejects_memory_only_and_reserved_input_before_reading_storage() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut live = AgentSession::start_for_test(backend, session()).unwrap();
    assert!(
        live.capture_fork_source(&UnexpectedForkReader)
            .unwrap_err()
            .to_string()
            .contains("durable history")
    );
    {
        let mut state = live.state.lock().unwrap();
        state.active_turn = Some(turn(1));
    }
    assert!(
        live.capture_fork_source(&UnexpectedForkReader)
            .unwrap_err()
            .to_string()
            .contains("pending input")
    );
    live.state.lock().unwrap().active_turn = None;
    let request = crate::ActivityRequestRef::new(
        activity(turn(1), 1),
        crate::RequestId::new(1.try_into().unwrap()),
    );
    live.state
        .lock()
        .unwrap()
        .outstanding_requests
        .insert(request);
    assert!(
        live.capture_fork_source(&UnexpectedForkReader)
            .unwrap_err()
            .to_string()
            .contains("pending input")
    );
    live.state.lock().unwrap().outstanding_requests.clear();
    live.shutdown().unwrap();
}

// durable idle 상태와 JoinHandle이 남아 있어도 실제 worker가 종료됐으면 저장소를 읽기 전에
// fork 캡처를 거부합니다. 명시적 shutdown으로 handle을 제거하지 않고 종료 경계를 검증합니다.
#[test]
fn fork_capture_rejects_a_finished_durable_idle_worker_before_reading_storage() {
    use crate::{JournalDurability, fixture_descriptor};

    let directory = TestDirectory::new();
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::Close,
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut live = AgentSession::start_cancellable_with_repository(
        backend,
        fixture_descriptor(session()),
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    while !live.worker.as_ref().unwrap().is_finished() {
        assert!(Instant::now() < deadline, "idle worker did not finish");
        // 초기 Changed를 소비해야 terminal Closed 전송이 가득 찬 change lane에서 풀립니다.
        live.poll().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        live.lifecycle.load(std::sync::atomic::Ordering::Acquire),
        super::super::WORKER_IDLE
    );
    assert!(live.state.lock().unwrap().active_turn.is_none());
    assert!(live.state.lock().unwrap().outstanding_requests.is_empty());
    assert!(matches!(
        live.transcript_reader().durability(),
        JournalDurability::Durable {
            journal_sequence: Some(_),
            ..
        }
    ));
    assert!(
        live.capture_fork_source(&UnexpectedForkReader)
            .unwrap_err()
            .to_string()
            .contains("idle live Session")
    );
    live.shutdown().unwrap();
}

// 실제 worker가 완료한 durable Anchor만 reader에서 캡처하며 반환한 replay와 live cutoff가
// 일치합니다. 부모 writer를 유지한 채 read-only reader로 검증합니다.
#[test]
fn fork_capture_accepts_the_actual_idle_durable_worker_source() {
    use crate::{
        BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
        BackendRequestEvidence, ContextPolicyChanged, ContextStrategy, ContinuationStrategy,
        ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ReplayExecutor,
        ReplayProfile, SubmissionId, session_repository::LocalSessionReader,
    };
    let directory = TestDirectory::new();
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let binding = BackendBindingEvidence::new(
        "managed",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "parent"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: ReplayProfile::SemanticOnly,
        },
    );
    let input = UserInput::new("capture this");
    let replay = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "capture this".to_owned(),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "durable answer".to_owned(),
            refusal: None,
        },
    ];
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession {
                session_id: session(),
            },
            evidence: BackendCommandEvidence::BindingOpened(binding),
        },
        BackendScriptStep::Emit(BackendEvent::ContextPolicyChanged {
            policy: ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65536),
            )
            .unwrap(),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: turn(1),
                input: input.clone(),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "request/v1",
                BackendIdentity::new("exchange/v1", "1"),
                BackendIdentity::new("accepted/v1", "1"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: turn(1),
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "outcome/v1",
                "1",
            ))
            .with_replay(ModelReplayDelta::new(
                Some(ModelReplayContract::new("system", vec![])),
                replay.clone(),
            )),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: turn(2),
                input: UserInput::new("later work"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "request/v1",
                BackendIdentity::new("exchange/v1", "2"),
                BackendIdentity::new("accepted/v1", "2"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: turn(2),
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "outcome/v1",
                "2",
            ))
            .with_replay(ModelReplayDelta::new(
                None,
                vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: "later work".into(),
                        refusal: None,
                    },
                    ModelReplayItem::Message {
                        role: ModelReplayRole::Assistant,
                        content: "later answer".into(),
                        refusal: None,
                    },
                ],
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut live = AgentSession::start_cancellable_with_repository(
        backend,
        crate::fixture_descriptor(session()),
        repository,
        || false,
    )
    .unwrap()
    .unwrap();
    let mut admission = live
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            input,
        )))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
        admission = live.retry(pending).unwrap();
    }
    live.wait_until_processed(1);
    live.wait_until_no_active_turn();
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    let captured = loop {
        match live.capture_fork_source(&reader) {
            Ok(captured) => break captured,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "capture did not become ready: {error}"
                );
                thread::sleep(Duration::from_millis(1));
            },
        }
    };
    assert_eq!(captured.descriptor().session_id(), session());
    assert_eq!(captured.target().model_replay().items(), replay);
    let crate::JournalDurability::Durable {
        journal_sequence, ..
    } = live.transcript_reader().durability()
    else {
        panic!("durable")
    };
    assert_eq!(captured.snapshot().journal_cutoff(), journal_sequence);
    let catalog = live
        .capture_fork_catalog(
            &reader,
            crate::session_repository::SessionForkLimits::default(),
        )
        .unwrap();
    let selected = catalog.selection(0).unwrap();
    assert_eq!(
        live.prepare_historical_fork_source(&selected)
            .unwrap()
            .target()
            .model_replay()
            .items(),
        replay
    );
    live.context_compaction_pending
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(live.prepare_historical_fork_source(&selected).is_err());
    live.context_compaction_pending
        .store(false, std::sync::atomic::Ordering::Release);
    {
        live.state.lock().unwrap().active_turn = Some(turn(2));
    }
    assert!(live.prepare_historical_fork_source(&selected).is_err());
    live.state.lock().unwrap().active_turn = None;
    let mut admission = live
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            UserInput::new("later work"),
        )))
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
        admission = live.retry(pending).unwrap();
    }
    live.wait_until_processed(2);
    live.wait_until_no_active_turn();
    assert!(live.prepare_historical_fork_source(&selected).is_err());
    let refreshed = loop {
        match live.capture_fork_catalog(
            &reader,
            crate::session_repository::SessionForkLimits::default(),
        ) {
            Ok(catalog) => break catalog,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "new catalog did not become ready: {error}"
                );
                thread::sleep(Duration::from_millis(1));
            },
        }
    };
    let earlier = refreshed.selection(1).unwrap();
    assert_eq!(
        live.prepare_historical_fork_source(&earlier)
            .unwrap()
            .target()
            .model_replay()
            .items(),
        replay
    );
    live.shutdown().unwrap();
    assert!(live.prepare_historical_fork_source(&earlier).is_err());
}
