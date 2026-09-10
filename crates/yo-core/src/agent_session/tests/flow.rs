use std::{
    thread,
    time::{Duration, Instant},
};

use super::{
    super::{AgentIntent, AgentSession, AgentSessionError},
    support::{activity, id, next_poll, session, start_app, turn},
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand,
    AgentControlOutcome, AgentEvent, ApprovalDecision, BackendCapabilities, BackendEvent,
    BackendFailure, BackendFailureKind, BackendScriptStep, CommandAdmission, InputSubmission,
    RequestId, RuntimeError, RuntimePoll, ScriptedBackend, SubmissionId, SubmissionOutcome,
    SubmissionRejection, SubmissionRejectionKind, TurnOutcome, UserInput,
};

fn wait_for_control_outcome(app: &mut AgentSession) -> AgentControlOutcome {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(outcome) = app.take_control_outcome() {
            return outcome;
        }
        assert!(
            Instant::now() < deadline,
            "test timed out waiting for a control outcome; poll={:?}",
            app.poll()
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_submission_outcome(app: &mut AgentSession) -> SubmissionOutcome {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(outcome) = app.take_submission_outcome() {
            return outcome;
        }
        assert!(Instant::now() < deadline, "submission outcome timed out");
        app.poll()
            .expect("submission rejection must keep the worker healthy");
        thread::sleep(Duration::from_millis(1));
    }
}

// 시작 입력이 거절되면 상관관계가 있는 결과를 돌려주고 예약을 해제하여 다음 입력이
// 새 Turn으로 실행된다. 거절된 입력은 Journal에 수락된 입력으로 남지 않는다.
#[test]
fn rejected_submission_releases_the_turn_and_allows_the_next_input() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::RejectCommand {
            command: AgentCommand::StartTurn {
                turn: turn(1),
                input: UserInput::from("reject"),
            },
            failure: BackendFailure::new(BackendFailureKind::CommandRejected, "input unavailable"),
        },
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: turn(2),
            input: UserInput::from("retry"),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: turn(2),
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);
    let submission = InputSubmission::new(SubmissionId::new().unwrap(), UserInput::from("reject"));
    let rejected_id = submission.id();
    let reader = app.transcript_reader();
    let head = reader.head_sequence();
    app.dispatch(AgentIntent::Submit(submission)).unwrap();
    assert_eq!(
        wait_for_submission_outcome(&mut app),
        SubmissionOutcome::Rejected {
            id: rejected_id,
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::Incompatible,
                "input unavailable"
            ),
        }
    );
    assert_eq!(reader.head_sequence(), head);
    app.dispatch(AgentIntent::submit("retry".to_owned()).unwrap())
        .unwrap();
    assert!(matches!(
        wait_for_submission_outcome(&mut app),
        SubmissionOutcome::Accepted { .. }
    ));
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: turn(2) })
    );
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: turn(2),
            outcome: TurnOutcome::Completed
        })
    );
    app.shutdown().unwrap();
}

// 참조 admission은 backend 전달과 Journal 기록 전에 실행된다. 거절 뒤에도 같은 Session에서
// 재시도할 수 있고, 실행 환경의 admission host는 첫 입력 이후 교체할 수 없다.
#[test]
fn reference_admission_rejects_before_dispatch_and_preserves_session_retry() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::{
        InputAdmissionConfigurationError, InputAdmissionHost, InputReference, WorkspaceReference,
        WorkspaceReferenceKind,
    };

    struct ReferenceHost(Arc<AtomicBool>);
    impl InputAdmissionHost for ReferenceHost {
        fn validate(&self, _: &UserInput) -> Result<(), SubmissionRejection> {
            if self.0.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(SubmissionRejection::new(
                    SubmissionRejectionKind::StaleReference,
                    "selected file disappeared",
                ))
            }
        }
    }
    let input = UserInput::with_references(
        "@a",
        vec![InputReference::workspace(
            0..2,
            WorkspaceReference::new(
                "file:a",
                "host:one",
                "workspace:one",
                "root:one",
                "a",
                WorkspaceReferenceKind::File,
            )
            .unwrap(),
        )],
    )
    .unwrap();
    for configured in [false, true] {
        let retry_input = if configured {
            input.clone()
        } else {
            UserInput::from("retry")
        };
        let backend = ScriptedBackend::new([
            BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
                session_id: session(),
            }),
            BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
                turn: turn(2),
                input: retry_input.clone(),
            }),
            BackendScriptStep::Emit(BackendEvent::TurnFinished {
                turn: turn(2),
                outcome: TurnOutcome::Completed,
            }),
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        let available = Arc::new(AtomicBool::new(false));
        let mut app = start_app(backend);
        if configured {
            app.configure_input_admission(Box::new(ReferenceHost(Arc::clone(&available))))
                .unwrap();
            assert_eq!(
                app.configure_input_admission(Box::new(ReferenceHost(Arc::clone(&available)))),
                Err(InputAdmissionConfigurationError::AlreadyConfigured)
            );
        }
        let reader = app.transcript_reader();
        let head = reader.head_sequence();
        let rejected_id = SubmissionId::new().unwrap();
        app.dispatch(AgentIntent::Submit(InputSubmission::new(
            rejected_id,
            input.clone(),
        )))
        .unwrap();
        let SubmissionOutcome::Rejected { id, rejection } = wait_for_submission_outcome(&mut app)
        else {
            panic!("reference must be rejected before backend dispatch")
        };
        assert_eq!(id, rejected_id);
        assert_eq!(
            rejection.kind(),
            if configured {
                SubmissionRejectionKind::StaleReference
            } else {
                SubmissionRejectionKind::EnvironmentUnavailable
            }
        );
        assert_eq!(reader.head_sequence(), head);
        assert_eq!(
            app.configure_input_admission(Box::new(ReferenceHost(Arc::clone(&available)))),
            Err(InputAdmissionConfigurationError::InputAlreadySubmitted)
        );
        available.store(true, Ordering::Release);
        let retry_id = SubmissionId::new().unwrap();
        app.dispatch(AgentIntent::Submit(InputSubmission::new(
            retry_id,
            retry_input,
        )))
        .unwrap();
        assert_eq!(
            wait_for_submission_outcome(&mut app),
            SubmissionOutcome::Accepted { id: retry_id }
        );
        assert_eq!(
            next_poll(&mut app).unwrap(),
            RuntimePoll::Event(AgentEvent::TurnStarted { turn: turn(2) })
        );
        assert_eq!(
            next_poll(&mut app).unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished {
                turn: turn(2),
                outcome: TurnOutcome::Completed,
            })
        );
        app.shutdown().unwrap();
    }
}

// core의 미지원 steer와 provider의 정상적인 steer 거절 모두 기존 Turn을 유지한다.
// 이후 중단 명령이 원래 Turn에 전달되고 같은 Session에서 새 작업을 시작할 수 있다.
#[test]
fn rejected_steer_preserves_the_active_turn_and_session() {
    for provider_rejects in [false, true] {
        let mut steps = vec![
            BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
                session_id: session(),
            }),
            BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
                turn: turn(1),
                input: UserInput::from("work"),
            }),
        ];
        if provider_rejects {
            steps.push(BackendScriptStep::RejectCommand {
                command: AgentCommand::SteerTurn {
                    turn: turn(1),
                    input: UserInput::from("adjust"),
                },
                failure: BackendFailure::new(
                    BackendFailureKind::CommandRejected,
                    "steer unavailable",
                ),
            });
        }
        steps.extend([
            BackendScriptStep::AcceptCommand(AgentCommand::InterruptTurn { turn: turn(1) }),
            BackendScriptStep::Emit(BackendEvent::TurnFinished {
                turn: turn(1),
                outcome: TurnOutcome::Interrupted,
            }),
            BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
                turn: turn(2),
                input: UserInput::from("next"),
            }),
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        let capabilities = if provider_rejects {
            BackendCapabilities::none().with_steer()
        } else {
            BackendCapabilities::none()
        };
        let mut app = start_app(ScriptedBackend::new(steps).with_capabilities(capabilities));
        app.dispatch(AgentIntent::submit("work".to_owned()).unwrap())
            .unwrap();
        assert!(matches!(
            wait_for_submission_outcome(&mut app),
            SubmissionOutcome::Accepted { .. }
        ));
        next_poll(&mut app).unwrap();
        let submission =
            InputSubmission::new(SubmissionId::new().unwrap(), UserInput::from("adjust"));
        let rejected_id = submission.id();
        app.dispatch(AgentIntent::Steer {
            turn: turn(1),
            submission,
        })
        .unwrap();
        assert!(
            matches!(wait_for_submission_outcome(&mut app), SubmissionOutcome::Rejected { id, rejection }
            if id == rejected_id && rejection.kind() == SubmissionRejectionKind::Incompatible)
        );
        app.dispatch(AgentIntent::Interrupt).unwrap();
        app.wait_until_no_active_turn();
        app.dispatch(AgentIntent::submit("next".to_owned()).unwrap())
            .unwrap();
        assert!(matches!(
            wait_for_submission_outcome(&mut app),
            SubmissionOutcome::Accepted { .. }
        ));
        app.shutdown().unwrap();
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

// 수동 압축을 적용할 수 없는 정상적인 경계는 Session failure가 아니다. frontend는
// typed control 결과를 받고 같은 Session에서 다음 prompt를 계속 실행할 수 있다.
#[test]
fn keeps_the_session_healthy_after_manual_compaction_is_rejected() {
    for asynchronous in [false, true] {
        let first = turn(1);
        let compact = AgentCommand::CompactContext {
            guidance: Some("keep unresolved constraints".to_owned()),
        };
        let rejection = BackendFailure::new(
            BackendFailureKind::CommandRejected,
            "compaction did not reduce the current context",
        );
        let mut steps = vec![BackendScriptStep::AcceptCommand(
            AgentCommand::CreateSession {
                session_id: session(),
            },
        )];
        if asynchronous {
            steps.push(BackendScriptStep::AcceptCommand(compact.clone()));
            steps.push(BackendScriptStep::Fail(rejection.clone()));
        } else {
            steps.push(BackendScriptStep::RejectCommand {
                command: compact.clone(),
                failure: rejection.clone(),
            });
        }
        steps.extend([
            BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
                turn: first,
                input: UserInput::from("continue"),
            }),
            BackendScriptStep::Emit(BackendEvent::TurnFinished {
                turn: first,
                outcome: TurnOutcome::Completed,
            }),
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        let mut app = start_app(ScriptedBackend::new(steps));

        assert_eq!(
            app.dispatch(AgentIntent::CompactContext {
                guidance: Some("keep unresolved constraints".to_owned()),
            })
            .unwrap(),
            CommandAdmission::Queued
        );
        app.wait_until_processed(1);
        assert_eq!(
            wait_for_control_outcome(&mut app),
            AgentControlOutcome::ContextCompactionRejected {
                detail: rejection.message().to_owned(),
            }
        );

        assert_eq!(
            app.dispatch(AgentIntent::submit("continue").unwrap())
                .unwrap(),
            CommandAdmission::Queued
        );
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
        app.shutdown().unwrap();
    }
}

// Engine-level validation is an expected control rejection too. Invalid guidance must not
// close the worker before the frontend can correct it and continue in the same Session.
#[test]
fn keeps_the_session_healthy_after_invalid_compaction_guidance() {
    let first = turn(1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("continue"),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    assert_eq!(
        app.dispatch(AgentIntent::CompactContext {
            guidance: Some(" leading whitespace".to_owned()),
        })
        .unwrap(),
        CommandAdmission::Queued
    );
    app.wait_until_processed(1);
    assert_eq!(
        wait_for_control_outcome(&mut app),
        AgentControlOutcome::ContextCompactionRejected {
            detail: "InvalidContextCompactionGuidance".to_owned(),
        }
    );

    assert_eq!(
        app.dispatch(AgentIntent::submit("continue").unwrap())
            .unwrap(),
        CommandAdmission::Queued
    );
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
    app.shutdown().unwrap();
}

// A prompt reserves its Turn before TurnStarted reaches the TUI. A compact command admitted in
// that window resolves as a notice instead of turning the expected scheduling race into failure.
#[test]
fn keeps_the_session_healthy_when_compaction_races_a_queued_turn() {
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
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: second,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    assert_eq!(
        app.dispatch(AgentIntent::submit("first").unwrap()).unwrap(),
        CommandAdmission::Queued
    );
    assert_eq!(
        app.dispatch(AgentIntent::CompactContext { guidance: None })
            .unwrap(),
        CommandAdmission::Queued
    );
    assert_eq!(
        wait_for_control_outcome(&mut app),
        AgentControlOutcome::ContextCompactionRejected {
            detail: "context compaction requires an idle Session".to_owned(),
        }
    );
    app.wait_until_processed(1);
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
        app.dispatch(AgentIntent::submit("second").unwrap())
            .unwrap(),
        CommandAdmission::Queued
    );
    app.wait_until_processed(2);
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnStarted { turn: second })
    );
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: second,
            outcome: TurnOutcome::Completed,
        })
    );
    app.shutdown().unwrap();
}

// frontend가 idle compaction을 queue한 뒤 binding replacement를 요청해도 replacement가
// 먼저 실행될 수 없다. 후보 backend는 정리되고 기존 Session이 compaction 결과를 소유한다.
#[test]
fn prevents_binding_replacement_from_overtaking_idle_compaction() {
    let compact = AgentCommand::CompactContext { guidance: None };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::RejectCommand {
            command: compact,
            failure: BackendFailure::new(
                BackendFailureKind::CommandRejected,
                "there is not enough history to compact",
            ),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = start_app(backend);

    assert_eq!(
        app.dispatch(AgentIntent::CompactContext { guidance: None })
            .unwrap(),
        CommandAdmission::Queued
    );
    let candidate = ScriptedBackend::new([BackendScriptStep::Shutdown(Ok(()))]);
    let error = app
        .replace_backend(Box::new(candidate), || false)
        .expect_err("replacement cannot pass the compaction reservation");
    assert!(matches!(
        error,
        AgentSessionError::WorkerUnavailable(ref detail)
            if detail.contains("cannot overtake pending context compaction")
    ));

    app.wait_until_processed(1);
    assert!(matches!(
        app.take_control_outcome(),
        Some(AgentControlOutcome::ContextCompactionRejected { .. })
    ));
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
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::StaleReference,
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
    for mode in 0..3 {
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
                response: if mode == 2 {
                    ActivityResponse::PreviousQuestion {
                        choice: Some(2),
                        draft: UserInput::from("the answer"),
                    }
                } else if mode == 1 {
                    ActivityResponse::QuestionAnswer {
                        choice: 2,
                        notes: UserInput::from("the answer"),
                    }
                } else {
                    ActivityResponse::UserInput(UserInput::from("the answer"))
                },
            }),
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        let mut app = start_app(backend);
        app.dispatch(AgentIntent::submit("ask me".to_owned()).unwrap())
            .unwrap();
        next_poll(&mut app).unwrap();
        next_poll(&mut app).unwrap();

        app.dispatch(if mode == 2 {
            AgentIntent::PreviousQuestion {
                request,
                choice: Some(2),
                draft: "the answer".to_owned(),
            }
        } else if mode == 1 {
            AgentIntent::RespondToQuestion {
                request,
                choice: 2,
                notes: "the answer".to_owned(),
            }
        } else {
            AgentIntent::RespondToUserInput {
                request,
                input: "the answer".to_owned(),
            }
        })
        .unwrap();
        app.wait_until_processed(2);

        app.shutdown().unwrap();
    }
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

// host가 스킬 본문을 누락하거나 다른 신원으로 돌려주면 실행·기록 전에 거절한다.
// caller가 미리 붙인 본문도 신뢰하지 않으며 올바른 host snapshot으로 재시도할 수 있다.
#[test]
fn skill_preparation_requires_matching_host_snapshot_before_dispatch() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::{
        InputAdmissionHost, InputReference, ResolvedSkill, SkillReference, SkillReferenceScope,
    };
    struct Host {
        mode: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        skill: SkillReference,
    }
    impl InputAdmissionHost for Host {
        fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
            assert_eq!(input.references()[0].skill_reference(), Some(&self.skill));
            Ok(())
        }
        fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
            self.validate(input)?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.mode.load(Ordering::SeqCst) {
                0 => Ok(None),
                1 => Ok(Some(
                    ResolvedSkill::new(
                        SkillReference::new(
                            "other",
                            "host",
                            "/other",
                            "other",
                            SkillReferenceScope::User,
                            1,
                            "revision",
                        ),
                        "wrong body",
                    )
                    .unwrap(),
                )),
                _ => Ok(Some(
                    ResolvedSkill::new(self.skill.clone(), "verified instructions").unwrap(),
                )),
            }
        }
    }
    let skill = SkillReference::new(
        "review",
        "host",
        "/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let draft =
        UserInput::with_references("$review", vec![InputReference::skill(0..7, skill.clone())])
            .unwrap();
    let resolved = draft
        .clone()
        .with_resolved_skill(ResolvedSkill::new(skill.clone(), "verified instructions").unwrap())
        .unwrap();
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: turn(4),
            input: resolved.clone(),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: turn(4),
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mode = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut app = start_app(backend);
    app.configure_input_admission(Box::new(Host {
        mode: Arc::clone(&mode),
        calls: Arc::clone(&calls),
        skill,
    }))
    .unwrap();
    let reader = app.transcript_reader();
    let head = reader.head_sequence();
    for index in 0..3 {
        mode.store(index, Ordering::SeqCst);
        let input = if index == 2 {
            resolved.clone()
        } else {
            draft.clone()
        };
        let id = SubmissionId::new().unwrap();
        app.dispatch(AgentIntent::Submit(InputSubmission::new(id, input)))
            .unwrap();
        let SubmissionOutcome::Rejected {
            id: observed,
            rejection,
        } = wait_for_submission_outcome(&mut app)
        else {
            panic!("invalid skill admission must reject")
        };
        assert_eq!(observed, id);
        assert_eq!(
            rejection.kind(),
            if index == 0 {
                SubmissionRejectionKind::RequiredAssetUnavailable
            } else {
                SubmissionRejectionKind::InvalidReference
            }
        );
        assert_eq!(reader.head_sequence(), head);
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "caller snapshot rejected before host preparation"
    );
    app.dispatch(AgentIntent::Submit(InputSubmission::new(
        SubmissionId::new().unwrap(),
        draft,
    )))
    .unwrap();
    assert!(matches!(
        wait_for_submission_outcome(&mut app),
        SubmissionOutcome::Accepted { .. }
    ));
    app.wait_until_no_active_turn();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    app.shutdown().unwrap();
}
