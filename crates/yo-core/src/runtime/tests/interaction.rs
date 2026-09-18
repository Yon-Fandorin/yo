use super::{activity, id, runtime_with_active_turn, session, submission, turn};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent,
    AgentRejection, AgentRuntime, ApprovalDecision, BackendCapabilities, BackendEvent,
    BackendScriptStep, RequestId, RuntimeError, RuntimePoll, ScriptedBackend, UserInput,
};

fn runtime_with_secret_request(
    response_steps: impl IntoIterator<Item = BackendScriptStep>,
) -> (AgentRuntime<ScriptedBackend>, ActivityRequestRef) {
    let active_turn = turn(session(1), 1);
    let request_activity = activity(active_turn, 1);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    let steps = [
        vec![
            BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                activity: request_activity,
                kind: ActivityKind::UserInputRequest { request_id },
            }),
            BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                activity: request_activity,
                outcome: ActivityOutcome::Completed,
            }),
        ],
        response_steps.into_iter().collect(),
    ]
    .concat();
    let (mut runtime, _) = runtime_with_active_turn(steps);
    runtime.poll_event().unwrap();
    runtime.poll_event().unwrap();
    (runtime, request)
}

// approval 요청과 사용자 응답이 steer나 새 Turn으로 바뀌지 않고 하나의 상관관계 흐름으로
// 정상 완료되는지 확인한다.
#[test]
fn completes_one_correlated_approval_cycle() {
    let active_turn = turn(session(1), 1);
    let request_activity = activity(active_turn, 1);
    let response_activity = activity(active_turn, 2);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    let response_command = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::Approval(ApprovalDecision::Approved),
    };
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: request_activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::AcceptCommand(response_command.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: response_activity,
            kind: ActivityKind::ApprovalResponse { request_id },
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: response_activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: crate::TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            kind: ActivityKind::ApprovalRequest { .. },
            ..
        })
    ));
    runtime.poll_event().unwrap();
    assert!(
        runtime
            .execute_command(response_command)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            kind: ActivityKind::ApprovalResponse { .. },
            ..
        })
    ));
    runtime.poll_event().unwrap();
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: crate::TurnOutcome::Completed,
        })
    );
    assert_eq!(runtime.active_turn(), None);
    runtime.shutdown().unwrap();
}

// steer를 지원하지 않는 backend에서는 core가 명령을 명시적으로 거절하고 backend script를
// 소비하지 않는지 확인한다.
#[test]
fn unsupported_steer_never_reaches_the_backend() {
    let active_turn = turn(session(1), 1);
    let steps = [BackendScriptStep::Shutdown(Ok(()))];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let error = runtime
        .execute_submission(
            AgentCommand::SteerTurn {
                turn: active_turn,
                input: UserInput::from("change direction"),
            },
            submission(2),
        )
        .unwrap_err();

    assert_eq!(
        error,
        RuntimeError::CommandRejected(AgentRejection::UnsupportedSteer)
    );
    assert_eq!(runtime.backend().remaining_steps(), 1);
    assert_eq!(runtime.active_turn(), Some(active_turn));
    runtime.shutdown().unwrap();
}

// steer capability가 확정된 backend에서는 같은 명령을 수락하되 Queue나 새 Turn을 만들지
// 않는지 확인한다.
#[test]
fn supported_steer_is_forwarded_without_creating_a_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::from("inspect"),
    };
    let steer = AgentCommand::SteerTurn {
        turn: active_turn,
        input: UserInput::from("focus on tests"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::AcceptCommand(steer.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_steer());
    let mut runtime = AgentRuntime::new(backend);
    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission(3)).unwrap();

    let events = runtime.execute_submission(steer, submission(4)).unwrap();

    assert!(events.is_empty());
    assert_eq!(runtime.active_turn(), Some(active_turn));
    assert_eq!(runtime.backend().remaining_steps(), 1);
    runtime.shutdown().unwrap();
}

// 미리 해석된 스킬이 포함된 질문 응답은 backend와 journal에 도달하지 않는다.
// 거절 후 같은 요청에 일반 응답을 보낼 수 있어 질문 상태도 소비하지 않는다.
#[test]
fn resolved_skills_in_activity_responses_never_reach_the_backend() {
    use crate::{InputReference, ResolvedSkill, SkillReference, SkillReferenceScope};
    let active_turn = turn(session(1), 1);
    let request_activity = activity(active_turn, 1);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    let plain = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::UserInput(UserInput::new("answer")),
    };
    let (mut runtime, _) = runtime_with_active_turn([
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        }),
        BackendScriptStep::AcceptCommand(plain.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    runtime.poll_event().unwrap();
    let reference = SkillReference::new(
        "skill:review",
        "host:one",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let input = UserInput::with_references(
        "$review",
        vec![InputReference::skill(0..7, reference.clone())],
    )
    .unwrap()
    .with_resolved_skill(ResolvedSkill::new(reference, "instructions").unwrap())
    .unwrap();
    for response in [
        ActivityResponse::UserInput(input.clone()),
        ActivityResponse::QuestionAnswer {
            choice: 1,
            notes: input.clone(),
        },
        ActivityResponse::PreviousQuestion {
            choice: None,
            draft: input,
        },
    ] {
        let error = runtime
            .execute_command(AgentCommand::RespondToActivity { request, response })
            .unwrap_err();
        assert!(matches!(error, RuntimeError::InputRejected(_)), "{error}");
        assert_eq!(runtime.backend().remaining_steps(), 2);
    }
    runtime.execute_command(plain).unwrap();
    assert_eq!(runtime.backend().remaining_steps(), 1);
    runtime.shutdown().unwrap();
}

// 비밀 입력은 백엔드에 정확한 바이트로 전달되지만 transcript에는 값 없는 영수증만 남는지 확인한다.
#[test]
fn secret_input_reaches_the_backend_exactly_but_only_a_receipt_is_committed() {
    use crate::{SecretInput, TranscriptRecord};

    let secret = "한글\nexact\0🙂";
    let request_activity = activity(turn(session(1), 1), 1);
    let request = ActivityRequestRef::new(request_activity, RequestId::new(id(1)));
    let live = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(secret).unwrap()),
    };
    let (mut runtime, observed_request) = runtime_with_secret_request([
        BackendScriptStep::AcceptCommand(live.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    assert_eq!(observed_request, request);

    runtime.execute_command(live).unwrap();

    let records = runtime.transcript_reader().read_after(None);
    assert!(records.entries().iter().any(|entry| {
        matches!(
            entry.record(),
            TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
                request: committed_request,
                response: ActivityResponse::SecretInputSubmitted,
            }) if *committed_request == request
        )
    }));
    assert!(!format!("{records:?}").contains(secret));
    runtime.shutdown().unwrap();
}

// 비밀 전송 실패가 백엔드의 원문 대신 고정된 공개 진단만 반환하는지 확인한다.
#[test]
fn failed_secret_dispatch_exposes_only_static_public_diagnostics() {
    use crate::SecretInput;

    let secret = "must-never-echo";
    let request = ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
    let live = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(secret).unwrap()),
    };
    let backend_failure = crate::BackendFailure::new(
        crate::BackendFailureKind::Protocol,
        format!("backend echoed {secret}"),
    );
    let (mut runtime, _) = runtime_with_secret_request([
        BackendScriptStep::RejectCommand {
            command: live.clone(),
            failure: backend_failure,
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let error = runtime.execute_command(live).unwrap_err();

    let RuntimeError::Backend { failure, .. } = error else {
        panic!("secret transport failure must remain a backend failure");
    };
    assert_eq!(failure.kind(), crate::BackendFailureKind::Protocol);
    assert_eq!(
        failure.message(),
        "secret input delivery failed with an unknown outcome"
    );
    assert!(!format!("{failure:?}").contains(secret));
    runtime.shutdown().unwrap();
}

// secret 응답을 backend가 수락한 뒤 poll이 비밀을 포함한 실패를 반환해도 반환 오류와
// 이미 기록된 durable transcript가 backend 진단을 다시 노출하지 않는지 확인한다.
#[test]
fn secret_poll_failure_redacts_backend_diagnostics_from_error_and_transcript() {
    use crate::SecretInput;

    let secret = "poll-secret-canary";
    let request = ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
    let live = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(secret).unwrap()),
    };
    let failure = crate::BackendFailure::new(
        crate::BackendFailureKind::Protocol,
        format!("poll backend echoed {secret}"),
    );
    let (mut runtime, _) = runtime_with_secret_request([
        BackendScriptStep::AcceptCommand(live.clone()),
        BackendScriptStep::Fail(failure),
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    runtime.execute_command(live).unwrap();
    let error = runtime.poll_event().unwrap_err();
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(secret), "{diagnostic}");

    let transcript = runtime.transcript_reader().read_after(None);
    let transcript_debug = format!("{transcript:?}");
    assert!(!transcript_debug.contains(secret), "{transcript_debug}");
    runtime.shutdown().unwrap();
}

// backend가 secret request binding을 이미 제거한 뒤 실패 Turn event를 보내더라도 core의
// process-local 보호 상태가 backend 진단을 공개 event나 durable transcript에 남기지 않는다.
#[test]
fn secret_turn_failure_event_is_redacted_after_request_cleanup() {
    use crate::{Failure, SecretInput, TurnOutcome};

    let secret = "turn-event-secret-canary";
    let active_turn = turn(session(1), 1);
    let request = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));
    let live = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(secret).unwrap()),
    };
    let (mut runtime, _) = runtime_with_secret_request([
        BackendScriptStep::AcceptCommand(live.clone()),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Failed(Failure::new(format!("backend echoed {secret}"))),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    runtime.execute_command(live).unwrap();
    let RuntimePoll::Event(AgentEvent::TurnFinished { outcome, .. }) =
        runtime.poll_event().unwrap()
    else {
        panic!("expected the redacted terminal Turn event");
    };
    let diagnostic = format!("{outcome:?}");
    assert!(!diagnostic.contains(secret), "{diagnostic}");
    assert!(matches!(
        outcome,
        TurnOutcome::Failed(ref failure)
            if failure.message()
                == "backend operation failed after protected input dispatch; details are withheld"
    ));

    let transcript = runtime.transcript_reader().read_after(None);
    let transcript_debug = format!("{transcript:?}");
    assert!(!transcript_debug.contains(secret), "{transcript_debug}");
    runtime.shutdown().unwrap();
}

// secret 응답을 backend가 수락한 뒤 shutdown이 비밀을 포함한 실패를 반환해도 반환 오류와
// durable transcript가 backend 진단을 다시 노출하지 않는지 확인한다.
#[test]
fn secret_shutdown_failure_redacts_backend_diagnostics_from_error_and_transcript() {
    use crate::SecretInput;

    let secret = "shutdown-secret-canary";
    let request = ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
    let live = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(secret).unwrap()),
    };
    let failure = crate::BackendFailure::new(
        crate::BackendFailureKind::Cleanup,
        format!("shutdown backend echoed {secret}"),
    );
    let (mut runtime, _) = runtime_with_secret_request([
        BackendScriptStep::AcceptCommand(live.clone()),
        BackendScriptStep::Shutdown(Err(failure)),
    ]);

    runtime.execute_command(live).unwrap();
    let error = runtime.shutdown().unwrap_err();
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(secret), "{diagnostic}");

    let transcript = runtime.transcript_reader().read_after(None);
    let transcript_debug = format!("{transcript:?}");
    assert!(!transcript_debug.contains(secret), "{transcript_debug}");
}

// backend가 secret batch를 초과했다고 거절하면 payload-free InputRejected를 반환하고 receipt를
// 기록하지 않으며, 같은 outstanding request를 더 작은 secret으로 재시도할 수 있는지 확인한다.
#[test]
fn over_budget_secret_rejection_is_safe_and_retryable() {
    use crate::{SecretInput, SubmissionRejectionKind, TranscriptRecord};

    let canary = "over-budget-secret-canary";
    let smaller_secret = "retry-secret";
    let request = ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
    let oversized = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(canary).unwrap()),
    };
    let retry = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new(smaller_secret).unwrap()),
    };
    let (mut runtime, _) = runtime_with_secret_request([
        BackendScriptStep::RejectCommand {
            command: oversized.clone(),
            failure: crate::BackendFailure::new(
                crate::BackendFailureKind::InputOverBudget,
                format!("backend retained {canary}"),
            ),
        },
        BackendScriptStep::AcceptCommand(retry.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let error = runtime.execute_command(oversized).unwrap_err();
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(canary), "{diagnostic}");
    assert!(matches!(
        error,
        RuntimeError::InputRejected(ref rejection)
            if rejection.kind() == SubmissionRejectionKind::OverBudget
                && rejection.message() == "secret interview input exceeds the 256 KiB live batch limit"
    ));
    let rejected_transcript = runtime.transcript_reader().read_after(None);
    assert!(!rejected_transcript.entries().iter().any(|entry| {
        matches!(
            entry.record(),
            TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
                response: ActivityResponse::SecretInputSubmitted,
                ..
            })
        )
    }));
    let rejected_transcript_debug = format!("{rejected_transcript:?}");
    assert!(!rejected_transcript_debug.contains(canary));

    runtime.execute_command(retry).unwrap();
    let retried_transcript = runtime.transcript_reader().read_after(None);
    assert!(retried_transcript.entries().iter().any(|entry| {
        matches!(
            entry.record(),
            TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
                request: committed_request,
                response: ActivityResponse::SecretInputSubmitted,
            }) if *committed_request == request
        )
    }));
    runtime.shutdown().unwrap();
}

// 값 없는 비밀 제출 영수증을 실제 응답처럼 다시 전송할 수 없는지 확인한다.
#[test]
fn a_secret_receipt_cannot_be_dispatched_as_a_live_response() {
    let (mut runtime, request) = runtime_with_secret_request([BackendScriptStep::Shutdown(Ok(()))]);

    let error = runtime
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInputSubmitted,
        })
        .unwrap_err();

    assert!(matches!(error, RuntimeError::InputRejected(_)));
    assert_eq!(runtime.backend().remaining_steps(), 1);
    runtime.shutdown().unwrap();
}

// Native protected input differs from legacy backend-owned interviews: backend acceptance only
// prepares transport, so a memory-only Journal cannot authorize it or leave the Session reusable.
#[test]
fn prepared_native_secret_requires_a_durable_receipt_and_terminalizes_the_session() {
    use crate::{BackendCommandEvidence, SecretInput};

    let active_turn = turn(session(1), 1);
    let request = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));
    let live = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::SecretInput(SecretInput::new("native-secret").unwrap()),
    };
    let (mut runtime, _) = runtime_with_secret_request([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: live.clone(),
            evidence: BackendCommandEvidence::ProtectedInputPrepared,
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let error = runtime.execute_command(live).unwrap_err();
    assert!(matches!(error, RuntimeError::Backend { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("receipt could not be committed durably"),
        "{error:?}"
    );
    let later = runtime
        .execute_submission(
            AgentCommand::SteerTurn {
                turn: active_turn,
                input: UserInput::from("continue"),
            },
            submission(9),
        )
        .unwrap_err();
    assert!(
        later
            .to_string()
            .contains("ended at a protected input submission")
    );
    runtime.shutdown().unwrap();
}
