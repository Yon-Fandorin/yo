use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent, AgentIntent,
    BackendAdapter, BackendCapabilities, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendPoll, BackendResumeTarget, BackendScriptStep, BackendStopHandle, DurabilityGapCause,
    JournalDurability, ScriptedBackend, SessionId, TranscriptRecord, TurnOutcome, UserInput,
    session_repository::{
        AppendError, AppendReceipt, DurableCutoff, DurableRecord, RepositoryEntry, RepositoryError,
        RepositorySequence, SessionRepository, StoragePressure, StoragePressureCause,
    },
};
use yo_tui::{AgentConnection, AgentPoll};

use super::support::{
    NeverTerminated, TEST_DEADLOCK_GUARD, collect_until, dispatch_until_queued, session_id, turn,
};
use crate::agent::TuiAgentConnection;

fn drain_frontend_until_turn_finished<E: std::fmt::Display>(
    timeout: Duration,
    mut poll: impl FnMut() -> Result<AgentPoll, E>,
) -> Result<Vec<AgentPoll>, String> {
    let deadline = Instant::now() + timeout;
    let mut observations = Vec::new();
    loop {
        let observation = poll().map_err(|error| error.to_string())?;
        let last_poll = match &observation {
            AgentPoll::Pending => "pending",
            AgentPoll::Submission(_) => "submission",
            AgentPoll::Record(_) => "record",
            AgentPoll::RequestTrace(_) => "request-trace",
            AgentPoll::Durability(_) => "durability",
            AgentPoll::Closed => "closed",
        };
        match observation {
            AgentPoll::Pending | AgentPoll::Submission(_) => thread::yield_now(),
            AgentPoll::Closed => {
                return Err(format!(
                    "connection closed before draining observations; collected={observations:?}"
                ));
            },
            observation => {
                let finished = matches!(
                    observation,
                    AgentPoll::Record(TranscriptRecord::EventCommitted(
                        AgentEvent::TurnFinished { .. }
                    ))
                );
                observations.push(observation);
                if finished {
                    return Ok(observations);
                }
            },
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out draining frontend observations; last_poll={last_poll}; collected={observations:?}"
            ));
        }
    }
}

struct CapacityPressureRepository;

/// Signals only when the backend is polled *after* the declared events were returned.
/// The intervening Runtime return and worker loop must therefore append the final event and
/// publish its change before the next poll can send the completion acknowledgment.
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

impl BackendAdapter for CompletionSignalingBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

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

// completion fixture는 마지막 event를 반환한 같은 poll에서는 신호하지 않고 다음 poll에
// 진입할 때만 신호해 Runtime append와 worker publication 사이의 barrier를 보존합니다.
#[test]
fn completion_signal_is_deferred_until_the_poll_after_the_last_event() {
    let (mut backend, completed) = CompletionSignalingBackend::new(
        ScriptedBackend::new([BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })]),
        1,
    );

    assert!(matches!(backend.poll_event(), Ok(BackendPoll::Event(_))));
    assert!(matches!(
        completed.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(backend.poll_event(), Ok(BackendPoll::Pending));
    completed
        .try_recv()
        .expect("the synchronous poll after the final event publishes completion");
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

    dispatch_until_queued(
        &mut connection,
        AgentIntent::submit("inspect".to_owned()).unwrap(),
    )
    .unwrap();
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

    let deadline = Instant::now() + TEST_DEADLOCK_GUARD;
    loop {
        match connection.poll().unwrap() {
            AgentPoll::Durability(JournalDurability::Gap {
                durable_cutoff: DurableCutoff::KnownEmpty,
                cause: DurabilityGapCause::Capacity,
            }) => break,
            AgentPoll::Pending
            | AgentPoll::Record(_)
            | AgentPoll::RequestTrace(_)
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
    let (backend, completed) = CompletionSignalingBackend::new(
        ScriptedBackend::new([
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
        ]),
        1,
    );
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start_persistent(
        backend,
        session_descriptor(),
        RecoveringPressureRepository::default(),
        &mut termination,
    )
    .unwrap()
    .unwrap();
    dispatch_until_queued(
        &mut connection,
        AgentIntent::submit("inspect".to_owned()).unwrap(),
    )
    .unwrap();

    // Backend completion is the ordering boundary. This longer timeout is only a finite
    // deadlock guard for cold hooks that compile and test several Rust targets concurrently.
    completed
        .recv_timeout(TEST_DEADLOCK_GUARD)
        .expect("worker did not finish the scripted Turn");
    assert_eq!(
        connection.transcript.head_sequence().map(|head| head.get()),
        Some(5),
        "backend completion must make the complete Journal prefix observable"
    );

    let observations =
        drain_frontend_until_turn_finished(TEST_DEADLOCK_GUARD, || connection.poll()).unwrap();

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

// Backend completion 뒤 frontend가 영구 Pending이면 observation ordering assertion 전에
// 독립 deadline과 마지막 poll class를 가진 focused failure로 끝납니다.
#[test]
fn completed_backend_with_stuck_frontend_poll_fails_within_the_guard() {
    let started = Instant::now();
    let error = drain_frontend_until_turn_finished(Duration::from_millis(5), || {
        Ok::<_, std::convert::Infallible>(AgentPoll::Pending)
    })
    .unwrap_err();

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(error.contains("last_poll=pending"));
    assert!(error.contains("collected=[]"));
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
    dispatch_until_queued(
        &mut connection,
        AgentIntent::submit("inspect".to_owned()).unwrap(),
    )
    .unwrap();

    completed
        .recv_timeout(TEST_DEADLOCK_GUARD)
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
