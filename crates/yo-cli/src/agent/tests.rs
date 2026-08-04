use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityRef, ActivityUpdate, AgentBackend, AgentCommand, AgentEvent,
    AgentIntent, BackendCapabilities, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendFailureKind, BackendPoll, BackendScriptStep, BackendStopHandle, CommandAdmission,
    DurabilityGapCause, JournalDurability, ScriptedBackend, SessionId, TranscriptRecord, TurnId,
    TurnOutcome, TurnRef, UserInput,
    session_repository::{
        AppendError, AppendReceipt, DurableCutoff, DurableRecord, RepositoryEntry, RepositoryError,
        RepositorySequence, SessionRepository, StoragePressure, StoragePressureCause,
    },
};
use yo_tui::{AgentConnection, AgentPoll, TerminationEvent, TerminationSource};

use super::TuiAgentConnection;

struct NeverTerminated;

struct CapacityPressureRepository;

struct CompletionSignalingBackend {
    inner: ScriptedBackend,
    remaining_events: usize,
    completion: Option<mpsc::Sender<()>>,
}

impl CompletionSignalingBackend {
    fn new(inner: ScriptedBackend, event_count: usize) -> (Self, mpsc::Receiver<()>) {
        let (completion, completed) = mpsc::channel();
        (
            Self {
                inner,
                remaining_events: event_count,
                completion: Some(completion),
            },
            completed,
        )
    }
}

impl AgentBackend for CompletionSignalingBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        self.inner.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.inner.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if self.remaining_events == 0
            && let Some(completion) = self.completion.take()
        {
            completion
                .send(())
                .expect("the completion receiver remains alive");
        }
        let poll = self.inner.poll_event()?;
        if matches!(poll, BackendPoll::Event(_)) {
            self.remaining_events = self
                .remaining_events
                .checked_sub(1)
                .expect("the script emits no undeclared event");
        }
        Ok(poll)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.inner.shutdown()
    }
}

#[derive(Clone, Default)]
struct RecoveringPressureRepository {
    state: Arc<Mutex<Vec<RepositoryEntry>>>,
    attempts: Arc<Mutex<usize>>,
}

impl SessionRepository for RecoveringPressureRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        let mut state = self.state.lock().unwrap();
        if *attempts == 2 {
            return Err(AppendError::StoragePressure {
                pressure: StoragePressure::new(
                    DurableCutoff::Unknown,
                    StoragePressureCause::Capacity,
                ),
                source: None,
            });
        }
        let sequence = RepositorySequence::new(u64::try_from(state.len()).unwrap() + 1);
        state.push(RepositoryEntry::new(sequence, record));
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
            .state
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

impl SessionRepository for CapacityPressureRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        _record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        Err(AppendError::StoragePressure {
            pressure: StoragePressure::new(
                DurableCutoff::KnownEmpty,
                StoragePressureCause::Capacity,
            ),
            source: None,
        })
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        _sequence: Option<RepositorySequence>,
        _limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        Ok(Vec::new())
    }
}

impl TerminationSource for NeverTerminated {
    fn poll_termination(&mut self) -> TerminationEvent {
        TerminationEvent::None
    }
}

fn session_id() -> SessionId {
    "01890f00-0000-7000-8000-000000000001"
        .parse()
        .expect("the fixture is a UUIDv7")
}

fn session_descriptor() -> yo_core::SessionDescriptor {
    yo_core::SessionDescriptor::for_session(
        session_id(),
        "10000000-0000-4000-8000-000000000001"
            .parse()
            .expect("the test Host fixture is a UUIDv4"),
        yo_core::HostWorkspacePath::normalize_local("/")
            .expect("the filesystem root is a canonical workspace path"),
    )
}

fn turn() -> TurnRef {
    TurnRef::new(session_id(), TurnId::new(NonZeroU64::MIN))
}

fn collect_until(
    connection: &mut TuiAgentConnection,
    done: impl Fn(&[TranscriptRecord]) -> bool,
) -> Result<Vec<TranscriptRecord>, String> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut records = Vec::new();
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(record)) => {
                records.push(record);
                if done(&records) {
                    return Ok(records);
                }
            },
            Ok(AgentPoll::Pending | AgentPoll::Submission(_)) if Instant::now() < deadline => {
                thread::yield_now();
            },
            Ok(other) => {
                return Err(format!(
                    "connection ended before the expected records: {other:?}"
                ));
            },
            Err(error) => return Err(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for committed records".to_owned());
        }
    }
}

// 로컬 CLI 어댑터는 변경 알림 자체를 화면에 보내지 않고 저널에서 명령과 이벤트를
// 확정된 순서대로 읽어 TUI 레코드로 전달한다.
#[test]
fn exposes_committed_commands_and_events_in_journal_order() {
    let create = AgentCommand::CreateSession {
        session_id: session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: turn(),
        input: UserInput::from("inspect"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::Emit(yo_core::BackendEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();

    assert_eq!(
        connection
            .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
            .unwrap(),
        CommandAdmission::Queued
    );
    let records = collect_until(&mut connection, |records| {
        records.iter().any(|record| {
            matches!(
                record,
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { .. })
            )
        })
    })
    .unwrap();

    assert_eq!(
        records,
        [
            TranscriptRecord::CommandCommitted(create),
            TranscriptRecord::EventCommitted(AgentEvent::SessionCreated {
                session_id: session_id(),
            }),
            TranscriptRecord::CommandCommitted(start),
            TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn: turn() }),
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                turn: turn(),
                outcome: TurnOutcome::Completed,
            }),
        ]
    );
    connection.shutdown().unwrap();
}

// startup의 첫 durable append부터 용량 압력이 발생해도 CLI 연결은 내부 query로만 남기지
// 않고 TUI가 소비할 typed durability 사건을 전달해야 한다. cutoff가 KnownEmpty인 것도
// 보존해야 사용자가 "저장된 것이 없음"과 "확인 불가"를 구분할 수 있다.
#[test]
fn exposes_initial_storage_pressure_to_the_connected_frontend() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session_id(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start_persistent(
        backend,
        session_descriptor(),
        CapacityPressureRepository,
        &mut termination,
    )
    .unwrap()
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match connection.poll().unwrap() {
            AgentPoll::Durability(JournalDurability::Gap {
                durable_cutoff: DurableCutoff::KnownEmpty,
                cause: DurabilityGapCause::Capacity,
            }) => break,
            AgentPoll::Pending
            | AgentPoll::Record(_)
            | AgentPoll::Durability(_)
            | AgentPoll::Submission(_) => {},
            AgentPoll::Closed => panic!("connection closed before reporting storage pressure"),
        }
        assert!(
            Instant::now() < deadline,
            "storage pressure was not delivered"
        );
        thread::yield_now();
    }
    connection.shutdown().unwrap();
}

// frontend가 첫 poll을 늦게 해 Gap과 복구가 하나의 worker wake-up으로 합쳐져도 두 상태
// 전환은 사라지지 않아야 한다. 특히 volatile record보다 Gap이 먼저, 복구 뒤 record보다
// Durable이 먼저 전달되어 화면 계층이 각 record의 저장 여부를 추측하지 않게 한다.
#[test]
fn preserves_gap_and_recovery_before_the_frontend_first_polls() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session_id(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start_persistent(
        backend,
        session_descriptor(),
        RecoveringPressureRepository::default(),
        &mut termination,
    )
    .unwrap()
    .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();

    // hk runs several Rust suites concurrently; allow scheduler delay without weakening the
    // semantic head-sequence assertion that follows.
    let deadline = Instant::now() + Duration::from_secs(4);
    while connection.transcript.head_sequence().map(|head| head.get()) != Some(5) {
        assert!(
            Instant::now() < deadline,
            "worker did not complete the scripted Turn"
        );
        thread::yield_now();
    }

    let mut observations = Vec::new();
    while !observations.iter().any(|observation| {
        matches!(
            observation,
            AgentPoll::Record(TranscriptRecord::EventCommitted(
                AgentEvent::TurnFinished { .. }
            ))
        )
    }) {
        match connection.poll().unwrap() {
            AgentPoll::Pending | AgentPoll::Submission(_) => thread::yield_now(),
            AgentPoll::Closed => panic!("connection closed before draining observations"),
            observation => observations.push(observation),
        }
    }

    let gap = observations
        .iter()
        .position(|observation| {
            matches!(
                observation,
                AgentPoll::Durability(JournalDurability::Gap {
                    cause: DurabilityGapCause::Capacity,
                    ..
                })
            )
        })
        .expect("the capacity gap remains observable");
    let volatile_start = observations
        .iter()
        .position(|observation| {
            matches!(
                observation,
                AgentPoll::Record(TranscriptRecord::CommandCommitted(
                    AgentCommand::StartTurn { .. }
                ))
            )
        })
        .unwrap();
    let recovered = observations
        .iter()
        .rposition(|observation| {
            matches!(
                observation,
                AgentPoll::Durability(JournalDurability::Durable { .. })
            )
        })
        .expect("the recovery remains observable");
    let finished = observations
        .iter()
        .position(|observation| {
            matches!(
                observation,
                AgentPoll::Record(TranscriptRecord::EventCommitted(
                    AgentEvent::TurnFinished { .. }
                ))
            )
        })
        .unwrap();
    assert!(gap < volatile_start);
    assert!(volatile_start < recovered);
    assert!(recovered < finished);
    connection.shutdown().unwrap();
}

// frontend가 읽기 전에 한 알림으로 합쳐진 레코드가 한 번의 저널 읽기 상한인 256개를
// 넘어도, CLI 어댑터는 새 알림을 기다리지 않고 당시 head까지 모든 페이지를 이어 읽는다.
#[test]
fn drains_more_than_one_bounded_page_from_one_coalesced_wake() {
    const UPDATE_COUNT: usize = 260;
    const EXPECTED_RECORDS: usize = UPDATE_COUNT + 5;

    let activity = ActivityRef::new(turn(), ActivityId::new(NonZeroU64::new(1).unwrap()));
    let mut script = vec![
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session_id(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        }),
    ];
    script.extend((0..UPDATE_COUNT).map(|index| {
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(index.to_string()),
        })
    }));
    script.push(BackendScriptStep::Shutdown(Ok(())));

    let (backend, completed) =
        CompletionSignalingBackend::new(ScriptedBackend::new(script), UPDATE_COUNT + 1);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();

    completed
        .recv_timeout(Duration::from_secs(10))
        .expect("worker did not finish the scripted Journal suffix");
    assert_eq!(
        connection.transcript.head_sequence().map(|head| head.get()),
        Some(EXPECTED_RECORDS as u64),
        "worker completion must make the complete Journal suffix observable"
    );

    let records =
        collect_until(&mut connection, |records| records.len() == EXPECTED_RECORDS).unwrap();

    assert_eq!(records.len(), EXPECTED_RECORDS);
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(
                record,
                TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { .. })
            ))
            .count(),
        UPDATE_COUNT
    );
    connection.shutdown().unwrap();
}

// 백엔드 실패가 저널에 Turn 실패를 확정한 경우 CLI 어댑터는 그 레코드를 먼저 모두
// 전달한 뒤 연결 오류를 보고한다.
#[test]
fn drains_committed_failure_record_before_reporting_connection_failure() {
    let create = AgentCommand::CreateSession {
        session_id: session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: turn(),
        input: UserInput::from("inspect"),
    };
    let backend_failure =
        BackendFailure::new(BackendFailureKind::ProcessExit, "provider stream stopped");
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create),
        BackendScriptStep::AcceptCommand(start),
        BackendScriptStep::Fail(backend_failure),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut saw_failed_turn = false;
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Failed(_),
                ..
            }))) => saw_failed_turn = true,
            Ok(AgentPoll::Record(_) | AgentPoll::Pending | AgentPoll::Submission(_)) => {},
            Ok(other) => panic!("connection closed without its failure: {other:?}"),
            Err(error) => {
                assert!(saw_failed_turn);
                assert!(error.to_string().contains("provider stream stopped"));
                break;
            },
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the connection failure"
        );
        thread::yield_now();
    }
    connection.shutdown().unwrap();
}

// 활성 Turn의 후속 명령이 backend에서 거절되면 worker는 cleanup이 확정한 Turn 종료를
// 먼저 공개하고, 그 다음에 명령 실패를 연결 오류로 보고한다.
#[test]
fn drains_cleanup_record_before_reporting_command_failure() {
    let create = AgentCommand::CreateSession {
        session_id: session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: turn(),
        input: UserInput::from("inspect"),
    };
    let steer = AgentCommand::SteerTurn {
        turn: turn(),
        input: UserInput::from("focus"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create),
        BackendScriptStep::AcceptCommand(start),
        BackendScriptStep::RejectCommand {
            command: steer.clone(),
            failure: BackendFailure::new(BackendFailureKind::Turn, "steer was rejected"),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_steer());
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();
    collect_until(&mut connection, |records| {
        records.iter().any(|record| {
            matches!(
                record,
                TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { .. })
            )
        })
    })
    .unwrap();
    connection
        .dispatch(AgentIntent::submit("focus".to_owned()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut records = Vec::new();
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(record)) => records.push(record),
            Ok(AgentPoll::Pending | AgentPoll::Submission(_)) => {},
            Ok(other) => panic!("connection closed without its failure: {other:?}"),
            Err(error) => {
                assert!(error.to_string().contains("steer was rejected"));
                break;
            },
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the command failure"
        );
        thread::yield_now();
    }

    assert!(!records.contains(&TranscriptRecord::CommandCommitted(steer)));
    assert!(records.iter().any(|record| {
        matches!(
            record,
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Interrupted,
                ..
            })
        )
    }));
    connection.shutdown().unwrap();
}
