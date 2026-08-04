use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU8, AtomicU64},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    super::{
        AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll, AgentWorker, ChangeLane,
        CommandAdmission, PendingCommand, SessionState, WORKER_IDLE, WorkerSignal, apply_event,
    },
    support::{next_poll, session, start_app, turn},
};
use crate::{
    AgentBackend, AgentCommand, AgentEvent, BackendCapabilities, BackendCommandEvidence,
    BackendFailure, BackendFailureKind, BackendPoll, BackendScriptStep, BackendStopHandle,
    RuntimeError, RuntimePoll, ScriptedBackend, SubmissionOutcome, SubmissionRejectionKind,
    TurnOutcome, UserInput, journal::SessionJournal,
};

// urgent interrupt가 normal lane에 이미 queue된 steer보다 먼저 실행되면 worker가 그
// steer를 Backend에 보내지 않고 같은 ID의 TargetChanged 거절을 실제 outcome lane에 남긴다.
#[test]
fn interrupt_rejects_an_already_queued_submission_with_its_exact_identity() {
    let active_turn = turn(1);
    let canceled_id = crate::SubmissionId::new().unwrap();
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("start"),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::InterruptTurn { turn: active_turn }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let state = Arc::new(Mutex::new(SessionState {
        active_turn: Some(active_turn),
        turn_started: true,
        ..SessionState::default()
    }));
    let active_turn_id = Arc::new(AtomicU64::new(active_turn.turn_id().get().get()));
    let outcomes = Arc::new(Mutex::new(VecDeque::new()));
    let mut worker = AgentWorker::new(
        backend,
        session(),
        Arc::clone(&state),
        Arc::clone(&active_turn_id),
        SessionJournal::new(),
        Arc::clone(&outcomes),
        None,
    );
    worker.initialize().unwrap();
    worker
        .runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("start"),
            },
            crate::SubmissionId::new().unwrap(),
        )
        .unwrap();

    let (normal_tx, normal_rx) = mpsc::sync_channel(1);
    let (urgent_tx, urgent_rx) = mpsc::sync_channel(1);
    normal_tx
        .send(PendingCommand::from_submission(
            AgentCommand::SteerTurn {
                turn: active_turn,
                input: UserInput::from("canceled"),
            },
            canceled_id,
        ))
        .unwrap();
    urgent_tx
        .send(PendingCommand::from_command(AgentCommand::InterruptTurn {
            turn: active_turn,
        }))
        .unwrap();
    drop(normal_tx);
    drop(urgent_tx);
    let (changes_tx, _changes_rx) = mpsc::sync_channel(1);
    let mut changes = ChangeLane::new(changes_tx, Arc::new(Mutex::new(None)));
    let processed = (Mutex::new(0), Condvar::new());
    let lifecycle = AtomicU8::new(WORKER_IDLE);

    let exit = worker.run(normal_rx, urgent_rx, &mut changes, &processed, &lifecycle);

    assert!(exit.failure.is_none());
    assert_eq!(
        outcomes.lock().unwrap().pop_front(),
        Some(SubmissionOutcome::Rejected {
            id: canceled_id,
            rejection: crate::SubmissionRejection::new(
                SubmissionRejectionKind::TargetChanged,
                "the target Turn was interrupted before this submission ran",
            ),
        })
    );
    assert!(outcomes.lock().unwrap().is_empty());
}

struct BlockingBackend {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    stop: mpsc::Sender<()>,
}

struct StartupBlockingBackend {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    stop: mpsc::Sender<()>,
}

struct ClosingCleanupBackend {
    release: mpsc::Receiver<()>,
    stop: mpsc::Sender<()>,
}

struct BlockingSteerBackend {
    commands: mpsc::Sender<AgentCommand>,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    blocked_once: bool,
}

impl AgentBackend for BlockingSteerBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        BackendStopHandle::no_op()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none().with_steer()
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.commands.send(command.clone()).unwrap();
        if matches!(
            command,
            AgentCommand::SteerTurn {
                ref input,
                ..
            } if input == &UserInput::from("block")
        ) && !self.blocked_once
        {
            self.blocked_once = true;
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        Ok(BackendPoll::Pending)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

impl AgentBackend for ClosingCleanupBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        let stop = self.stop.clone();
        BackendStopHandle::new(move || {
            let _ = stop.send(());
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn execute_command(
        &mut self,
        _command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        Ok(BackendPoll::Closed)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.release.recv().unwrap();
        Err(BackendFailure::new(
            BackendFailureKind::Cleanup,
            "racing cleanup failure",
        ))
    }
}

impl AgentBackend for StartupBlockingBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        let stop = self.stop.clone();
        BackendStopHandle::new(move || {
            let _ = stop.send(());
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn execute_command(
        &mut self,
        _command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        Ok(BackendPoll::Pending)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

impl AgentBackend for BlockingBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        let stop = self.stop.clone();
        BackendStopHandle::new(move || {
            let _ = stop.send(());
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if matches!(command, AgentCommand::StartTurn { .. }) {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        Ok(BackendPoll::Pending)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

// provider가 StartTurn acceptance에서 멈춰 있어도 dispatch와 poll은 UI 소유 스레드에서
// 즉시 돌아오고, worker가 풀린 뒤에만 TurnStarted event가 도착한다.
#[test]
fn provider_wait_never_blocks_the_frontend_connection() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let backend = BlockingBackend {
        entered: entered_tx,
        release: release_rx,
        stop: release_tx.clone(),
    };
    let mut app = start_app(backend);

    app.dispatch(AgentIntent::submit("wait".to_owned()).unwrap())
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(app.poll().unwrap(), AgentSessionPoll::Pending);

    release_tx.send(()).unwrap();
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { .. })
    ));
    app.shutdown().unwrap();
}

// provider acceptance가 끝나지 않은 상태에서 종료해도 별도 stop handle이 그 호출을 깨우므로
// frontend shutdown은 worker 뒤에서 무기한 기다리지 않는다.
#[test]
fn shutdown_stops_an_in_flight_provider_call() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let backend = BlockingBackend {
        entered: entered_tx,
        release: stop_rx,
        stop: stop_tx,
    };
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::submit("wait".to_owned()).unwrap())
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let started = Instant::now();
    app.shutdown().unwrap();

    assert!(started.elapsed() < Duration::from_secs(1));
}

// Session 생성 RPC가 멈춘 동안 host termination이 도착하면 startup wait가 stop handle을
// 호출하고 worker를 회수한 뒤, TUI에 진입하지 않았음을 None으로 돌려준다.
#[test]
fn host_termination_cancels_blocked_session_startup() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let backend = StartupBlockingBackend {
        entered: entered_tx,
        release: stop_rx,
        stop: stop_tx,
    };
    let started = Instant::now();
    let app = AgentSession::start_cancellable(backend, || true).unwrap();

    assert!(app.is_none());
    assert!(entered_rx.recv_timeout(Duration::from_secs(1)).is_ok());
    assert!(started.elapsed() < Duration::from_secs(1));
}

// worker가 provider 호출 안에서 runtime 상태를 전이하는 동안에도 frontend dispatch는
// 기다리지 않고 입력 당시 활성 Turn의 steer command를 단일 소비 token에 보존하며,
// 그 Turn이 끝난 뒤에는 새 Turn으로 재해석하지 않고 명시적으로 거부한다.
#[test]
fn retains_the_temporal_target_while_the_runtime_is_busy() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let backend = BlockingBackend {
        entered: entered_tx,
        release: stop_rx,
        stop: stop_tx.clone(),
    };
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::submit("wait".to_owned()).unwrap())
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let submission = crate::InputSubmission::new(
        crate::SubmissionId::new().unwrap(),
        UserInput::from("retain me"),
    );
    let submission_id = submission.id();
    let CommandAdmission::Backpressured(retained) =
        app.dispatch(AgentIntent::Submit(submission)).unwrap()
    else {
        panic!("a busy runtime must retain the identified command");
    };
    assert_eq!(retained.submission_id(), Some(submission_id));
    assert!(matches!(
        retained.command(),
        AgentCommand::SteerTurn { turn: target, input }
            if *target == turn(1) && input == &UserInput::from("retain me")
    ));

    let CommandAdmission::Backpressured(retained) = app.retry(retained).unwrap() else {
        panic!("retry while the worker is busy must retain the same submission");
    };
    assert_eq!(retained.submission_id(), Some(submission_id));

    stop_tx.send(()).unwrap();
    app.wait_until_processed(1);
    {
        let mut state = app.state.lock().unwrap();
        apply_event(
            &mut state,
            &app.active_turn_id,
            &AgentEvent::TurnFinished {
                turn: turn(1),
                outcome: TurnOutcome::Completed,
            },
        );
    }
    assert!(matches!(
        app.retry(retained),
        Err(AgentSessionError::TurnNoLongerActive)
    ));
    app.shutdown().unwrap();
}

// runtime 호출 중 보존된 steer보다 interrupt 재시도가 먼저 수용되면 interrupt barrier가
// 그 steer의 후속 재시도와 새 steer를 모두 거부해 stale command로 worker를 죽이지 않는다.
#[test]
fn accepted_interrupt_rejects_retained_and_new_steers() {
    let (commands_tx, commands_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let backend = BlockingSteerBackend {
        commands: commands_tx,
        entered: entered_tx,
        release: release_rx,
        blocked_once: false,
    };
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::submit("start".to_owned()).unwrap())
        .unwrap();
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { .. })
    ));

    app.dispatch(AgentIntent::submit("block".to_owned()).unwrap())
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let CommandAdmission::Backpressured(retained_steer) = app
        .dispatch(AgentIntent::submit("cancel one".to_owned()).unwrap())
        .unwrap()
    else {
        panic!("the busy runtime must retain the steer");
    };
    let CommandAdmission::Backpressured(mut retained_interrupt) =
        app.dispatch(AgentIntent::Interrupt).unwrap()
    else {
        panic!("the busy runtime must retain the interrupt");
    };
    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match app.retry(retained_interrupt).unwrap() {
            CommandAdmission::Queued => break,
            CommandAdmission::Backpressured(pending) => {
                assert!(
                    Instant::now() < deadline,
                    "the interrupt did not leave backpressure"
                );
                retained_interrupt = pending;
                thread::sleep(Duration::from_millis(1));
            },
        }
    }
    assert!(matches!(
        app.retry(retained_steer),
        Err(AgentSessionError::TurnInterruptPending)
    ));
    assert!(matches!(
        app.dispatch(AgentIntent::submit("too late".to_owned()).unwrap()),
        Err(AgentSessionError::TurnInterruptPending)
    ));
    app.wait_until_processed(3);
    app.shutdown().unwrap();

    let commands: Vec<_> = commands_rx.try_iter().collect();
    assert!(commands.iter().any(|command| matches!(
        command,
        AgentCommand::InterruptTurn { turn: command_turn } if *command_turn == turn(1)
    )));
    assert!(!commands.iter().any(|command| matches!(
        command,
        AgentCommand::SteerTurn { input, .. }
            if input == &UserInput::from("cancel one")
    )));
}

// backend가 스스로 닫히는 순간 cleanup도 실패하고 frontend 종료가 겹쳐도, 아직 poll되지
// 않은 cleanup failure를 shutdown이 change lane과 별도로 회수해 성공으로 숨기지 않는다.
#[test]
fn shutdown_retains_an_unpolled_worker_cleanup_failure() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::Close,
        BackendScriptStep::Shutdown(Err(BackendFailure::new(
            BackendFailureKind::Cleanup,
            "late cleanup failure",
        ))),
    ]);
    let mut app = start_app(backend);
    thread::sleep(Duration::from_millis(30));

    let error = app.shutdown().unwrap_err();

    assert!(matches!(
        error,
        AgentSessionError::Runtime(RuntimeError::Backend {
            ref failure,
            ..
        }) if failure.kind() == BackendFailureKind::Cleanup
    ));
}

// worker cleanup failure가 frontend의 event receiver drain과 정확히 겹쳐도 shared failure
// ledger가 queue 전달 여부와 독립적으로 원인을 보존해 shutdown 성공으로 바뀌지 않는다.
#[test]
fn shutdown_retains_a_cleanup_failure_that_races_with_receiver_drop() {
    let (stop_tx, stop_rx) = mpsc::channel();
    let backend = ClosingCleanupBackend {
        release: stop_rx,
        stop: stop_tx,
    };
    let mut app = start_app(backend);

    let error = app.shutdown().unwrap_err();

    assert!(matches!(
        error,
        AgentSessionError::Runtime(RuntimeError::Backend {
            ref failure,
            ..
        }) if failure.kind() == BackendFailureKind::Cleanup
    ));
}

// Journal 변경 알림을 frontend가 아직 읽지 않았을 때 후속 commit은 payload나 알림을
// 무한히 쌓지 않고 같은 level-triggered 신호 하나로 합쳐야 한다.
#[test]
fn coalesces_journal_changes_while_one_notification_is_unread() {
    let (sender, receiver) = mpsc::sync_channel(1);
    let mut lane = ChangeLane::new(sender, Arc::new(Mutex::new(None)));

    assert!(lane.changed());
    assert!(lane.changed());
    assert!(lane.changed());
    assert!(matches!(receiver.recv().unwrap(), WorkerSignal::Changed));
    assert!(receiver.try_recv().is_err());
}
