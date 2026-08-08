use super::{activity, runtime_with_active_turn, session, submission, turn};
use crate::{
    ActivityKind, AgentCommand, AgentRuntime, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendOutcomeEvidence, BackendRequestEvidence, BackendScriptStep, RuntimeError,
    ScriptedBackend, TurnOutcome, UserInput, journal::SemanticRecord,
};

// Runtime의 공개 경계도 Start/Steer와 SubmissionId를 분리해서 받을 수 없게 막아,
// backend 호출 전에 Journal이 표현할 수 없는 command correlation을 거절합니다.
#[test]
fn rejects_a_runtime_command_with_the_wrong_submission_correlation_shape() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let mut runtime =
        AgentRuntime::new(ScriptedBackend::new([BackendScriptStep::Shutdown(Ok(()))]));

    assert_eq!(
        runtime
            .execute_command(AgentCommand::StartTurn {
                turn,
                input: UserInput::new("missing identity"),
            })
            .unwrap_err(),
        RuntimeError::SubmissionIdentityRequired
    );
    assert_eq!(
        runtime
            .execute_submission(AgentCommand::CreateSession { session_id }, submission(10),)
            .unwrap_err(),
        RuntimeError::SubmissionIdentityUnexpected
    );
    assert!(runtime.journal().entries().is_empty());
    runtime.shutdown().unwrap();
}

// AgentSession을 우회해 공개 Runtime API를 직접 사용해도 이미 commit된 SubmissionId는
// backend와 Journal에 두 번째로 전달되지 않아 Session 단위 correlation이 유지됩니다.
#[test]
fn rejects_a_duplicate_submission_identity_at_the_runtime_boundary() {
    let (mut runtime, active_turn) =
        runtime_with_active_turn([BackendScriptStep::Shutdown(Ok(()))]);
    let before = runtime.journal().entries().len();

    let error = runtime
        .execute_submission(
            AgentCommand::SteerTurn {
                turn: active_turn,
                input: UserInput::new("duplicate"),
            },
            submission(1),
        )
        .expect_err("a committed SubmissionId cannot be reused");

    assert_eq!(
        error,
        RuntimeError::DuplicateSubmissionIdentity(submission(1))
    );
    assert_eq!(runtime.journal().entries().len(), before);
    assert_eq!(runtime.backend().remaining_steps(), 1);
    runtime.shutdown().unwrap();
}

// Runtime이 backend에서 수락되고 core에 commit된 명령만 Journal에 남겨야 하므로,
// 거절된 중단 명령은 기록하지 않고 뒤이어 수락된 같은 명령만 정확히 한 번 기록한다.
#[test]
fn records_only_commands_that_reach_semantic_commit() {
    let active_turn = turn(session(1), 1);
    let interrupt = AgentCommand::InterruptTurn { turn: active_turn };
    let failure = BackendFailure::new(BackendFailureKind::Turn, "interrupt rejected");
    let steps = [
        BackendScriptStep::RejectCommand {
            command: interrupt.clone(),
            failure: failure.clone(),
        },
        BackendScriptStep::AcceptCommand(interrupt.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    let before = runtime.journal().entries().len();

    assert_eq!(
        runtime.execute_command(interrupt.clone()).unwrap_err(),
        RuntimeError::Backend {
            failure,
            terminal_events: Vec::new(),
        }
    );
    assert_eq!(runtime.journal().entries().len(), before);

    assert!(
        runtime
            .execute_command(interrupt.clone())
            .unwrap()
            .is_empty()
    );
    assert_eq!(runtime.journal().entries().len(), before + 1);
    assert_eq!(
        runtime.journal().entries().last().unwrap().record(),
        &SemanticRecord::CommandCommitted(
            crate::journal::CommittedCommand::uncorrelated(interrupt).unwrap()
        )
    );
}

// backend event를 의미 상태에 commit한 뒤 frontend에 반환할 때 같은 AgentEvent가 Journal에도
// 먼저 남아야 이후 Transcript가 live 출력과 동일한 순서를 replay할 수 있다.
#[test]
fn records_committed_backend_events_before_they_are_observed() {
    let active_turn = turn(session(1), 1);
    let work = activity(active_turn, 1);
    let backend_event = BackendEvent::ActivityStarted {
        activity: work,
        kind: ActivityKind::ModelWork,
    };
    let steps = [
        BackendScriptStep::Emit(backend_event),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let observed = runtime.poll_event().unwrap();

    let crate::RuntimePoll::Event(event) = observed else {
        panic!("expected one committed runtime event");
    };
    assert_eq!(
        runtime.journal().entries().last().unwrap().record(),
        &SemanticRecord::EventCommitted(event)
    );
}

// backend failure가 활성 Activity와 Turn을 닫을 때 frontend에 돌려주는 terminal event
// 전체가 같은 순서로 Journal에도 남아야 실패 직전의 history가 조용히 잘리지 않는다.
#[test]
fn records_terminal_events_created_by_backend_failure() {
    let active_turn = turn(session(1), 1);
    let work = activity(active_turn, 1);
    let failure = BackendFailure::new(BackendFailureKind::ProcessExit, "backend exited");
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        }),
        BackendScriptStep::Fail(failure),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    runtime.poll_event().unwrap();
    let before_failure = runtime.journal().entries().len();

    let RuntimeError::Backend {
        terminal_events, ..
    } = runtime.poll_event().unwrap_err()
    else {
        panic!("expected a backend failure");
    };

    let recorded = &runtime.journal().entries()[before_failure..];
    assert!(
        !terminal_events.is_empty(),
        "an active Turn failure must create terminal events"
    );
    assert_eq!(recorded.len(), terminal_events.len());
    for (entry, event) in recorded.iter().zip(terminal_events) {
        assert_eq!(entry.record(), &SemanticRecord::EventCommitted(event));
    }
}

// 재개 가능한 backend가 Session과 요청의 실제 식별 증거를 반환하고 완료를 통지하면,
// Runtime은 command/event와 상관관계 레코드를 계약 순서로 묶고 Transcript에는 의미 기록만 보인다.
#[test]
fn records_a_complete_live_continuation_chain_without_exposing_it_in_transcript() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::new("continue later"),
    };
    let steps = [
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(binding_evidence()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: active_turn,
            evidence: BackendOutcomeEvidence::without_identity(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let mut runtime = AgentRuntime::new(ScriptedBackend::new(steps));
    let submission_id = submission(7);

    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission_id).unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(_)
    ));

    let entries = runtime.journal().entries();
    let SemanticRecord::BackendExchangeObserved(exchange) = entries[5].record() else {
        panic!("the accepted submission must record its outbound exchange");
    };
    let SemanticRecord::BackendRequestAccepted(accepted) = entries[6].record() else {
        panic!("the accepted request must follow its exchange");
    };
    let SemanticRecord::BackendResumableOutcome(outcome) = entries[8].record() else {
        panic!("the completed Turn must publish a resumable outcome");
    };
    let SemanticRecord::ContinuationAnchor(anchor) = entries[9].record() else {
        panic!("the continuation anchor must immediately follow the outcome");
    };
    assert_eq!(exchange.epoch(), 1);
    assert_eq!(exchange.operation_id().as_uuid(), submission_id.as_uuid());
    assert_eq!(accepted.operation_id().as_uuid(), submission_id.as_uuid());
    assert_eq!(accepted.exchange_sequence(), entries[5].sequence());
    assert_eq!(outcome.accepted_request_sequence(), entries[6].sequence());
    assert_eq!(anchor.resumable_outcome_sequence(), entries[8].sequence());
    assert_eq!(anchor.journal_boundary(), entries[8].sequence());

    let transcript = runtime.journal().transcript_reader().read_after(None);
    assert_eq!(transcript.entries().len(), 5);
    assert_eq!(transcript.head(), Some(entries[7].sequence()));
    runtime.shutdown().unwrap();
}

// binding이나 accepted request가 없는 backend가 재개 가능한 완료라고 주장하면 Runtime이
// Anchor를 만들지 않고 Protocol 실패로 Turn을 닫아 불확실한 요청을 재전송하지 않게 한다.
#[test]
fn rejects_resumable_completion_without_accepted_request_evidence() {
    let active_turn = turn(session(1), 1);
    let (mut runtime, _) = runtime_with_active_turn([
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: active_turn,
            evidence: BackendOutcomeEvidence::without_identity(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let error = runtime.poll_event().unwrap_err();
    let RuntimeError::Backend { failure, .. } = error else {
        panic!("missing correlation evidence must be a backend protocol failure");
    };
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(
        runtime
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.record(), SemanticRecord::ContinuationAnchor(_)))
    );
    runtime.shutdown().unwrap();
}

// backend 증거가 command와 맞지 않아 오류가 나더라도 사용자가 입력한 본문은 오류 문자열에
// 포함하지 않고 command 종류만 알려야 로그나 UI를 통해 prompt가 노출되지 않습니다.
#[test]
fn redacts_submission_input_from_incompatible_evidence_errors() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let secret = "private prompt that must stay redacted";
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::new(secret),
    };
    let steps = [
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let mut runtime = AgentRuntime::new(ScriptedBackend::new(steps));

    runtime.execute_command(create).unwrap();
    let message = runtime
        .execute_submission(start, submission(1))
        .unwrap_err()
        .to_string();

    assert!(message.contains("StartTurn"));
    assert!(!message.contains(secret));
    runtime.shutdown().unwrap();
}

// 증거가 있던 Start 뒤에 증거 없는 Steer가 수락되면 완료 Anchor가 과거 Start 증거를
// 재사용하지 않아야 마지막으로 수락된 요청을 증명할 수 없는 상태를 안전하게 거절합니다.
#[test]
fn does_not_reuse_older_request_evidence_after_an_uncorrelated_steer() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::new("first"),
    };
    let steer = AgentCommand::SteerTurn {
        turn: active_turn,
        input: UserInput::new("latest"),
    };
    let steps = [
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(binding_evidence()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::AcceptCommand(steer.clone()),
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: active_turn,
            evidence: BackendOutcomeEvidence::without_identity(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let backend =
        ScriptedBackend::new(steps).with_capabilities(BackendCapabilities::none().with_steer());
    let mut runtime = AgentRuntime::new(backend);

    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission(1)).unwrap();
    runtime.execute_submission(steer, submission(2)).unwrap();
    let RuntimeError::Backend { failure, .. } = runtime.poll_event().unwrap_err() else {
        panic!("an uncorrelated latest request must prevent a resumable completion");
    };

    assert!(failure.message().contains("without an accepted request"));
    assert!(
        runtime
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.record(), SemanticRecord::ContinuationAnchor(_)))
    );
    runtime.shutdown().unwrap();
}

// backend가 최신 Steer를 수락한 뒤 잘못된 evidence를 반환해 Runtime commit은 거절되어도,
// 이전 요청 증거를 남기지 않아 완료 시 과거 요청으로 거짓 Anchor를 만들지 않습니다.
#[test]
fn invalidates_older_request_evidence_after_a_malformed_steer_receipt() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::new("first"),
    };
    let steer = AgentCommand::SteerTurn {
        turn: active_turn,
        input: UserInput::new("latest"),
    };
    let malformed = BackendRequestEvidence::new(
        "scripted/request/v1",
        BackendIdentity::new("", "invalid-exchange"),
        BackendIdentity::new("scripted/request/v1", "invalid-request"),
    );
    let steps = [
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(binding_evidence()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: steer.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(malformed),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: active_turn,
            evidence: BackendOutcomeEvidence::without_identity(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let backend =
        ScriptedBackend::new(steps).with_capabilities(BackendCapabilities::none().with_steer());
    let mut runtime = AgentRuntime::new(backend);

    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission(1)).unwrap();
    let RuntimeError::Backend { failure, .. } = runtime
        .execute_submission(steer, submission(2))
        .unwrap_err()
    else {
        panic!("malformed latest request evidence must fail as a backend protocol error");
    };
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);

    let RuntimeError::Backend { failure, .. } = runtime.poll_event().unwrap_err() else {
        panic!("a rejected latest receipt must invalidate earlier request evidence");
    };
    assert!(failure.message().contains("without an accepted request"));
    assert!(
        runtime
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.record(), SemanticRecord::ContinuationAnchor(_)))
    );
    runtime.shutdown().unwrap();
}

// interrupted Turn이 끝난 뒤 늦은 resumable 완료가 오면 과거 accepted request를 찾지 못해야
// 종료된 Turn의 correlation 상태가 새 활성 Turn이나 오류 분류에 영향을 주지 않습니다.
#[test]
fn clears_accepted_request_evidence_when_a_turn_becomes_terminal() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::new("interrupt me"),
    };
    let steps = [
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(binding_evidence()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        }),
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: active_turn,
            evidence: BackendOutcomeEvidence::without_identity(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let mut runtime = AgentRuntime::new(ScriptedBackend::new(steps));

    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission(1)).unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(crate::AgentEvent::TurnFinished { .. })
    ));
    let RuntimeError::Backend { failure, .. } = runtime.poll_event().unwrap_err() else {
        panic!("a late completion must not reuse terminal Turn correlation state");
    };

    assert!(failure.message().contains("without an accepted request"));
    runtime.shutdown().unwrap();
}

fn binding_evidence() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "scripted",
        "1",
        BackendIdentity::new("scripted/binding/v1", "binding-1"),
        BackendIdentity::new("scripted/model/v1", "model-1"),
        BackendIdentity::new("scripted/session/v1", "session-1"),
        crate::ContinuationStrategy::BackendManagedState,
    )
}

fn request_evidence() -> BackendRequestEvidence {
    BackendRequestEvidence::new(
        "scripted/request/v1",
        BackendIdentity::new("scripted/exchange/v1", "exchange-1"),
        BackendIdentity::new("scripted/request/v1", "request-1"),
    )
}
