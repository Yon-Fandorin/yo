use std::{
    collections::HashMap,
    fs,
    num::NonZeroU64,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::{
    AgentIntent, AgentSession, AgentSessionError, CommandAdmission, EventLane, WorkerEvent,
};
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    ActivityUpdate, AgentBackend, AgentCommand, AgentEvent, ApprovalDecision, BackendCapabilities,
    BackendEvent, BackendFailure, BackendFailureKind, BackendPoll, BackendScriptStep,
    BackendStopHandle, CodexBackend, CodexBackendConfig, RequestId, RuntimeError, RuntimePoll,
    ScriptedBackend, SessionId, TurnId, TurnOutcome, TurnRef, UserInput,
};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn session() -> SessionId {
    SessionId::new(id(1))
}

fn turn(value: u64) -> TurnRef {
    TurnRef::new(session(), TurnId::new(id(value)))
}

fn activity(turn: TurnRef, value: u64) -> ActivityRef {
    ActivityRef::new(turn, ActivityId::new(id(value)))
}

fn start_app(backend: impl AgentBackend + Send + 'static) -> AgentSession {
    let mut app = AgentSession::start(backend).unwrap();
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::SessionCreated {
            session_id: session(),
        })
    );
    app
}

fn next_poll(app: &mut AgentSession) -> Result<RuntimePoll, AgentSessionError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match app.poll()? {
            RuntimePoll::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            },
            RuntimePoll::Pending => {
                return Err(AgentSessionError::WorkerUnavailable(
                    "test timed out waiting for a worker observation".to_owned(),
                ));
            },
            observation => return Ok(observation),
        }
    }
}

// 첫 prompt는 Session을 한 번 만든 뒤 Turn 1을 시작하고 backend의 완료 event를 그대로
// frontend poll 경계로 전달한다.
#[test]
fn starts_the_first_turn_and_forwards_completion() {
    let first = turn(1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    app.dispatch(AgentIntent::Submit("inspect".to_owned()))
        .unwrap();
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: first })
    );
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Completed,
        })
    );
    app.shutdown().unwrap();
}

// frontend가 첫 TurnFinished를 아직 poll하지 않았더라도 worker가 이미 완료를 적용했다면
// 다음 prompt는 끝난 Turn의 steer가 아니라 새 Turn으로 시작되어야 한다.
#[test]
fn starts_a_new_turn_before_the_frontend_polls_completion() {
    let first = turn(1);
    let second = turn(2);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("first"),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: second,
            input: UserInput::from("second"),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    app.dispatch(AgentIntent::Submit("first".to_owned()))
        .unwrap();
    app.wait_until_processed(1);
    app.wait_until_no_active_turn();
    app.dispatch(AgentIntent::Submit("second".to_owned()))
        .unwrap();
    app.wait_until_processed(2);

    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: first })
    );
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Completed,
        })
    );
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: second })
    );
    app.shutdown().unwrap();
}

// Turn이 실행 중일 때 제출한 prompt는 새 Turn이나 queue가 아니라 같은 Turn의 steer
// command로 전달된다.
#[test]
fn submits_active_turn_input_as_steer() {
    let first = turn(1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::SteerTurn {
            turn: first,
            input: UserInput::from("focus on tests"),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_steer());
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::Submit("inspect".to_owned()))
        .unwrap();
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: first })
    );

    app.dispatch(AgentIntent::Submit("focus on tests".to_owned()))
        .unwrap();
    app.wait_until_processed(2);
    app.shutdown().unwrap();
}

// approval 응답은 원래 Activity와 request ID를 보존한 core response command로 전달되어
// 일반 prompt 제출과 섞이지 않는다.
#[test]
fn correlates_an_approval_response_with_its_request() {
    let first = turn(1);
    let request_activity = activity(first, 1);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("edit"),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::Submit("edit".to_owned()))
        .unwrap();
    next_poll(&mut app).unwrap();
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            kind: ActivityKind::ApprovalRequest { .. },
            ..
        })
    ));

    app.dispatch(AgentIntent::RespondToApproval {
        request,
        decision: ApprovalDecision::Approved,
    })
    .unwrap();
    assert!(matches!(
        app.dispatch(AgentIntent::RespondToApproval {
            request,
            decision: ApprovalDecision::Approved,
        }),
        Err(AgentSessionError::NoOutstandingRequest)
    ));
    app.wait_until_processed(2);
    app.shutdown().unwrap();
}

// Ctrl+C에 해당하는 interrupt intent는 활성 Turn만 대상으로 하고 backend가 보낸
// Activity와 Turn의 Interrupted 완료를 poll에서 구분해 관찰한다.
#[test]
fn interrupts_only_the_active_turn() {
    let first = turn(1);
    let work = activity(first, 1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("long task"),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::InterruptTurn { turn: first }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: work,
            outcome: ActivityOutcome::Interrupted,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Interrupted,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::Submit("long task".to_owned()))
        .unwrap();
    next_poll(&mut app).unwrap();
    next_poll(&mut app).unwrap();

    app.dispatch(AgentIntent::Interrupt).unwrap();
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            outcome: ActivityOutcome::Interrupted,
            ..
        })
    ));
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            outcome: TurnOutcome::Interrupted,
            ..
        })
    ));
    app.shutdown().unwrap();
}

// 활성 Turn이 없는 interrupt는 임의의 Turn을 만들거나 backend에 명령을 보내지 않고
// queue에 넣기 전 제품 연결 경계에서 즉시 명확한 오류로 남는다.
#[test]
fn rejects_interrupt_without_an_active_turn() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    assert!(matches!(
        app.dispatch(AgentIntent::Interrupt),
        Err(AgentSessionError::NoActiveTurn)
    ));
    app.shutdown().unwrap();
}

// agent-requested input은 같은 ActivityRequestRef의 UserInput 응답으로 전달되고 활성
// Turn의 steer command로 재해석되지 않는다.
#[test]
fn correlates_agent_requested_input_instead_of_steering() {
    let first = turn(1);
    let request_activity = activity(first, 1);
    let request_id = RequestId::new(id(2));
    let request = ActivityRequestRef::new(request_activity, request_id);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("ask me"),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::UserInput(UserInput::from("the answer")),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::Submit("ask me".to_owned()))
        .unwrap();
    next_poll(&mut app).unwrap();
    next_poll(&mut app).unwrap();

    app.dispatch(AgentIntent::RespondToUserInput {
        request,
        input: "the answer".to_owned(),
    })
    .unwrap();
    app.wait_until_processed(2);

    app.shutdown().unwrap();
}

// Session 생성 실패 뒤 명시적 backend shutdown도 실패하면 두 RuntimeError를 하나도
// 숨기지 않고 서로 다른 Session과 Cleanup failure kind로 반환한다.
#[test]
fn retains_session_start_and_cleanup_failures() {
    let create = AgentCommand::CreateSession {
        session_id: session(),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::RejectCommand {
            command: create,
            failure: BackendFailure::new(BackendFailureKind::Session, "session creation failed"),
        },
        BackendScriptStep::Shutdown(Err(BackendFailure::new(
            BackendFailureKind::Cleanup,
            "child cleanup failed",
        ))),
    ]);

    let error = match AgentSession::start(backend) {
        Ok(_) => panic!("Session and cleanup failures must reject startup"),
        Err(error) => error,
    };

    let AgentSessionError::StartAndCleanup { start, cleanup } = error else {
        panic!("both startup and cleanup failures must be retained");
    };
    assert!(matches!(
        *start,
        RuntimeError::Backend { ref failure, .. }
            if failure.kind() == BackendFailureKind::Session
    ));
    assert!(matches!(
        *cleanup,
        RuntimeError::Backend { ref failure, .. }
            if failure.kind() == BackendFailureKind::Cleanup
    ));
}

// fake backend의 실행 중 failure는 worker에서 core의 typed RuntimeError가 된 뒤 frontend
// poll 오류로 전달되며 shutdown 단계와 섞여 사라지지 않는다.
#[test]
fn reports_a_fake_backend_turn_failure_through_the_product_connection() {
    let first = turn(1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("fail"),
        }),
        BackendScriptStep::Fail(BackendFailure::new(
            BackendFailureKind::Turn,
            "provider turn failed",
        )),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);
    app.dispatch(AgentIntent::Submit("fail".to_owned()))
        .unwrap();
    next_poll(&mut app).unwrap();

    assert!(matches!(
        next_poll(&mut app),
        Err(AgentSessionError::Runtime(RuntimeError::Backend {
            ref failure,
            ..
        })) if failure.kind() == BackendFailureKind::Turn
    ));
    app.shutdown().unwrap();
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

    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure> {
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
        Ok(())
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

    fn execute_command(&mut self, _command: AgentCommand) -> Result<(), BackendFailure> {
        Ok(())
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

    fn execute_command(&mut self, _command: AgentCommand) -> Result<(), BackendFailure> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        Ok(())
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

    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure> {
        if matches!(command, AgentCommand::StartTurn { .. }) {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        Ok(())
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

    app.dispatch(AgentIntent::Submit("wait".to_owned()))
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(app.poll().unwrap(), RuntimePoll::Pending);

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
    app.dispatch(AgentIntent::Submit("wait".to_owned()))
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
    app.dispatch(AgentIntent::Submit("wait".to_owned()))
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let CommandAdmission::Backpressured(retained) = app
        .dispatch(AgentIntent::Submit("retain me".to_owned()))
        .unwrap()
    else {
        panic!("a busy runtime must retain the identified command");
    };
    assert!(matches!(
        &retained.0,
        AgentCommand::SteerTurn { turn: target, input }
            if *target == turn(1) && input == &UserInput::from("retain me")
    ));

    stop_tx.send(()).unwrap();
    app.wait_until_processed(1);
    {
        let mut state = app.state.lock().unwrap();
        super::apply_event(
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
    app.dispatch(AgentIntent::Submit("start".to_owned()))
        .unwrap();
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { .. })
    ));

    app.dispatch(AgentIntent::Submit("block".to_owned()))
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let CommandAdmission::Backpressured(retained_steer) = app
        .dispatch(AgentIntent::Submit("cancel one".to_owned()))
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
            CommandAdmission::Accepted => break,
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
        app.dispatch(AgentIntent::Submit("too late".to_owned())),
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
// 않은 cleanup failure를 shutdown이 bounded event lane에서 회수해 성공으로 숨기지 않는다.
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

// bounded event lane이 가득 찼을 때 같은 Activity의 중간 TextSnapshot은 무한히 쌓지 않고
// 가장 최신 값 하나로 합치며, 빈자리가 생기면 그 최신 snapshot을 전달한다.
#[test]
fn coalesces_replaceable_snapshots_while_the_event_lane_is_full() {
    let work = activity(turn(1), 1);
    let (sender, receiver) = mpsc::sync_channel(1);
    let mut lane = EventLane::new(sender, Arc::new(Mutex::new(None)));
    let snapshot = |text: &str| {
        WorkerEvent::Event(AgentEvent::ActivityUpdated {
            activity: work,
            update: ActivityUpdate::TextSnapshot(text.to_owned()),
        })
    };

    assert!(lane.send(snapshot("first")));
    assert!(lane.send(snapshot("second")));
    assert!(lane.send(snapshot("latest")));
    assert!(matches!(
        receiver.recv().unwrap(),
        WorkerEvent::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(ref text),
            ..
        }) if text == "first"
    ));

    assert!(lane.flush_available());
    assert!(matches!(
        receiver.recv().unwrap(),
        WorkerEvent::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(ref text),
            ..
        }) if text == "latest"
    ));
}

// 호환되는 로컬 Codex와 인증 환경이 있을 때 실제 도구가 disposable workspace에 파일을
// 만들고 Tool, FileChange, 완료 Turn event가 모두 관찰되는지 환경 통합 경로로 확인한다.
#[test]
#[ignore = "requires compatible authenticated Codex and performs one model turn"]
fn local_codex_completes_a_real_file_change() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let workspace = std::env::temp_dir().join(format!(
        "yo-codex-file-change-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&workspace).unwrap();

    let result = run_local_file_change(&workspace);
    let cleanup = fs::remove_dir_all(&workspace);

    result.unwrap();
    cleanup.unwrap();
}

fn run_local_file_change(workspace: &std::path::Path) -> Result<(), String> {
    let backend = CodexBackend::spawn(CodexBackendConfig::new(workspace))
        .map_err(|error| error.to_string())?;
    let mut app = AgentSession::start(backend).map_err(|error| error.to_string())?;
    app.dispatch(AgentIntent::Submit(
        "First run `pwd` with the shell command tool and wait for it to complete. Then use the \
         file patch tool to create yo-proof.txt in the current workspace containing exactly \
         YO_CODEX_INTEGRATION_OK followed by one newline. Perform both actions, then stop."
            .to_owned(),
    ))
    .map_err(|error| error.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(180);
    let mut activities = HashMap::new();
    let mut completed_tool = false;
    let mut completed_file_change = false;
    let turn_outcome = loop {
        if Instant::now() >= deadline {
            break Err("Codex Turn did not complete within 180 seconds".to_owned());
        }
        match app.poll().map_err(|error| error.to_string())? {
            RuntimePoll::Pending => thread::sleep(Duration::from_millis(10)),
            RuntimePoll::Closed => {
                break Err("Codex closed before the Turn completed".to_owned());
            },
            RuntimePoll::Event(event) => match event {
                AgentEvent::ActivityStarted { activity, kind } => {
                    activities.insert(activity, kind);
                    if let ActivityKind::ApprovalRequest { request_id } = kind {
                        app.dispatch(AgentIntent::RespondToApproval {
                            request: ActivityRequestRef::new(activity, request_id),
                            decision: ApprovalDecision::Approved,
                        })
                        .map_err(|error| error.to_string())?;
                    }
                },
                AgentEvent::ActivityFinished { activity, outcome } => {
                    if outcome == ActivityOutcome::Completed {
                        match activities.get(&activity) {
                            Some(ActivityKind::ToolCall) => completed_tool = true,
                            Some(ActivityKind::FileChange) => completed_file_change = true,
                            _ => {},
                        }
                    }
                },
                AgentEvent::TurnFinished { outcome, .. } => break Ok(outcome),
                AgentEvent::SessionCreated { .. }
                | AgentEvent::TurnStarted { .. }
                | AgentEvent::ActivityUpdated { .. } => {},
            },
        }
    };
    let shutdown = app.shutdown().map_err(|error| error.to_string());
    let turn_outcome = turn_outcome?;
    shutdown?;

    if turn_outcome != TurnOutcome::Completed {
        return Err(format!("Codex Turn ended as {turn_outcome:?}"));
    }
    if !completed_tool {
        return Err("no completed Tool Activity was observed".to_owned());
    }
    if !completed_file_change {
        return Err("no completed FileChange Activity was observed".to_owned());
    }
    let content =
        fs::read_to_string(workspace.join("yo-proof.txt")).map_err(|error| error.to_string())?;
    if content != "YO_CODEX_INTEGRATION_OK\n" {
        return Err(format!("unexpected file content: {content:?}"));
    }
    Ok(())
}
