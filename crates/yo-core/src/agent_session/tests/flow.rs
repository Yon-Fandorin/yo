use super::{
    super::{AgentIntent, AgentSession, AgentSessionError},
    support::{activity, id, next_poll, session, start_app, turn},
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent,
    ApprovalDecision, BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind,
    BackendScriptStep, CommandAdmission, InputSubmission, RequestId, RuntimeError, RuntimePoll,
    ScriptedBackend, SubmissionId, SubmissionOutcome, TurnOutcome, UserInput,
};

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

    let submission = InputSubmission::new(SubmissionId::new().unwrap(), UserInput::from("inspect"));
    let submission_id = submission.id();
    app.dispatch(AgentIntent::Submit(submission)).unwrap();
    app.wait_until_processed(1);
    assert_eq!(
        app.take_submission_outcome(),
        Some(SubmissionOutcome::Accepted { id: submission_id })
    );
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

// 같은 SubmissionId를 다시 제출하면 두 번째 입력은 backend나 semantic Journal에 닿기
// 전에 동기적으로 거절되어, 이미 수락된 correlation identity를 재사용하지 않습니다.
#[test]
fn rejects_a_duplicate_submission_identity_before_backend_execution() {
    let first = turn(1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("first"),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);
    let submission_id = SubmissionId::new().unwrap();

    app.dispatch(AgentIntent::Submit(InputSubmission::new(
        submission_id,
        UserInput::from("first"),
    )))
    .unwrap();
    app.wait_until_processed(1);
    let error = app
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            submission_id,
            UserInput::from("duplicate"),
        )))
        .expect_err("a Session rejects a reused submission identity");

    assert!(matches!(
        error,
        AgentSessionError::DuplicateSubmissionId(id) if id == submission_id
    ));
    assert_eq!(
        app.take_submission_outcome(),
        Some(SubmissionOutcome::Accepted { id: submission_id })
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

    app.dispatch(AgentIntent::submit("first".to_owned()).unwrap())
        .unwrap();
    app.wait_until_processed(1);
    app.wait_until_no_active_turn();
    app.dispatch(AgentIntent::submit("second".to_owned()).unwrap())
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

// TUI가 Turn 1을 보고 만든 steer는 worker가 그 Turn을 이미 끝냈다면 새 Turn으로
// 재해석되지 않는다. poll 지연은 prompt의 원래 귀속을 바꿀 수 없다.
#[test]
fn rejects_an_exact_steer_after_its_observed_turn_finishes() {
    let first = turn(1);
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
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    app.dispatch(AgentIntent::submit("first".to_owned()).unwrap())
        .unwrap();
    app.wait_until_processed(1);
    app.wait_until_no_active_turn();
    let submission =
        InputSubmission::new(SubmissionId::new().unwrap(), UserInput::from("late steer"));
    let submission_id = submission.id();

    assert_eq!(
        app.dispatch(AgentIntent::Steer {
            turn: first,
            submission,
        })
        .unwrap(),
        CommandAdmission::Rejected {
            id: submission_id,
            rejection: crate::SubmissionRejection::new(
                crate::SubmissionRejectionKind::StaleReference,
                "the command's target Turn is no longer active",
            ),
        }
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
    app.dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: first })
    );

    app.dispatch(AgentIntent::submit("focus on tests".to_owned()).unwrap())
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
    app.dispatch(AgentIntent::submit("edit".to_owned()).unwrap())
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
    app.dispatch(AgentIntent::submit("long task".to_owned()).unwrap())
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
    app.dispatch(AgentIntent::submit("ask me".to_owned()).unwrap())
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

    let error = match AgentSession::start_for_test(backend, session()) {
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

// fake backend의 실행 중 failure가 만든 TurnFinished 의미는 Journal에서 먼저 읽히고,
// worker의 typed RuntimeError도 다음 poll 오류로 남아 shutdown과 섞여 사라지지 않는다.
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
    app.dispatch(AgentIntent::submit("fail".to_owned()).unwrap())
        .unwrap();
    next_poll(&mut app).unwrap();
    assert!(matches!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        })
    ));

    assert!(matches!(
        next_poll(&mut app),
        Err(AgentSessionError::Runtime(RuntimeError::Backend {
            ref failure,
            ..
        })) if failure.kind() == BackendFailureKind::Turn
    ));
    app.shutdown().unwrap();
}
