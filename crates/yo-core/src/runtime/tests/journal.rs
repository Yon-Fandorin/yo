#[cfg(test)]
use std::array;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use super::{activity, runtime_with_active_turn, session, submission, turn};
#[cfg(test)]
use crate::journal::{CommittedCommand, SessionJournal};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, ActivityUpdate,
    AgentCommand, AgentRuntime, BackendAdapter, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendOutcomeEvidence, BackendPoll, BackendRequestEvidence, BackendResumeTarget,
    BackendScriptStep, BackendStopHandle, ContextCheckpointProposal, ContextPolicyChanged,
    ContextStrategy, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ReplayExecutor, ReplayProfile, RequestId, RuntimeError, ScriptedBackend, TurnOutcome,
    UserInput,
    journal::SemanticRecord,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, RepositoryEntry, RepositoryError,
        RepositorySequence, SessionRepository,
    },
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
        &SemanticRecord::CommandCommitted(CommittedCommand::uncorrelated(interrupt).unwrap())
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

// runtime이 successor 요청 수락보다 checkpoint를 먼저 Journal에 commit하는 순서를 검증합니다.
#[test]
fn commits_a_checkpoint_before_accepting_the_successor_context_request() {
    let session_id = session(8);
    let turns = [
        turn(session_id, 1),
        turn(session_id, 2),
        turn(session_id, 3),
    ];
    let inputs = ["first input", "second input", "current input"];
    let starts = array::from_fn::<_, 3, _>(|index| AgentCommand::StartTurn {
        turn: turns[index],
        input: UserInput::new(inputs[index]),
    });
    let contract = ModelReplayContract::new("system", Vec::new());
    let group = |input: &str, answer: &str| {
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: input.to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: answer.to_owned(),
                refusal: None,
            },
        ]
    };
    let first_group = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: inputs[0].to_owned(),
            refusal: None,
        },
        ModelReplayItem::FunctionCall {
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        ModelReplayItem::FunctionCallOutput {
            call_id: "call-1".to_owned(),
            output: "identical artifact bytes".to_owned(),
        },
        ModelReplayItem::FunctionCall {
            call_id: "call-2".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        ModelReplayItem::FunctionCallOutput {
            call_id: "call-2".to_owned(),
            output: "identical artifact bytes".to_owned(),
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "first answer".to_owned(),
            refusal: None,
        },
    ];
    let second_group = group(inputs[1], "second answer");
    let steer_commands = [
        AgentCommand::SteerTurn {
            turn: turns[2],
            input: UserInput::new("first correction"),
        },
        AgentCommand::SteerTurn {
            turn: turns[2],
            input: UserInput::new("second correction"),
        },
    ];
    let active_group = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: inputs[2].to_owned(),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "first correction".to_owned(),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "second correction".to_owned(),
            refusal: None,
        },
    ];
    let policy = ContextPolicyChanged::try_new(
        1,
        true,
        ContextStrategy::PortableSummaryV1Alpha1,
        85,
        90,
        Some(10),
        Some(65_536),
    )
    .unwrap();
    let body = "# Context Checkpoint\n\
## Current Objective\nContinue.\n\
## Active Constraints\nNone.\n\
## Decisions\nRetain exact recent context.\n\
## Verified Progress\nThe first turn completed.\n\
## Current State\nThe third turn is active.\n\
## Unknown or Unverified\nNone.\n\
## Next Actions\nAnswer the current input.\n\
## Critical References\nNone.";
    let proposal = ContextCheckpointProposal::new(
        Some(turns[2]),
        1,
        100,
        90,
        40,
        contract.clone(),
        body,
        vec![first_group.clone()],
        vec![second_group.clone()],
        active_group.clone(),
        serde_json::json!({
            "schema": "yo.model-usage-receipt/v1",
            "response_id": "summary-1",
            "round": 1,
            "provider": "test",
            "account": "default",
            "model": "test-model",
            "connector": "openai-responses",
            "api_dialect": "openai-responses",
            "base_url": "https://example.invalid/",
            "usage": {
                "input_tokens": 20,
                "output_tokens": 10,
                "total_tokens": 30,
                "reasoning_tokens": 0
            },
            "cache_read_input_tokens": { "availability": "unsupported" }
        }),
    )
    .unwrap();
    let binding = exact_replay_binding_evidence();
    let create = AgentCommand::CreateSession { session_id };
    let first_delta = ModelReplayDelta::new(Some(contract), first_group);
    let second_delta = ModelReplayDelta::new(None, second_group);
    let final_delta = ModelReplayDelta::new(
        None,
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "final answer".to_owned(),
            refusal: None,
        }],
    );
    let steps = [
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(binding),
        },
        BackendScriptStep::Emit(BackendEvent::ContextPolicyChanged {
            policy: policy.clone(),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: starts[0].clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: turns[0],
            evidence: BackendOutcomeEvidence::without_identity().with_replay(first_delta),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: starts[1].clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: turns[1],
            evidence: BackendOutcomeEvidence::without_identity().with_replay(second_delta),
        }),
        BackendScriptStep::AcceptCommand(starts[2].clone()),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: steer_commands[0].clone(),
            evidence: BackendCommandEvidence::SubmissionPrepared,
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: steer_commands[1].clone(),
            evidence: BackendCommandEvidence::SubmissionPrepared,
        },
        BackendScriptStep::Emit(BackendEvent::ContextCheckpointPrepared { proposal }),
        BackendScriptStep::Emit(BackendEvent::ModelRequestAccepted {
            turn: turns[2],
            evidence: request_evidence(),
        }),
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: turns[2],
            evidence: BackendOutcomeEvidence::without_identity().with_replay(final_delta),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let mut journal = SessionJournal::with_repository_and_descriptor(
        Box::new(RuntimeTestRepository::default()),
        crate::fixture_descriptor(session_id),
    );
    journal.initialize_durability();
    let backend =
        ScriptedBackend::new(steps).with_capabilities(BackendCapabilities::none().with_steer());
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    let transcript = runtime.journal().transcript_reader();

    runtime.execute_command(create).unwrap();
    for (index, start) in starts.iter().take(2).enumerate() {
        runtime
            .execute_submission(start.clone(), submission(20 + index as u8))
            .unwrap();
        assert!(matches!(
            runtime.poll_event().unwrap(),
            crate::RuntimePoll::Event(_)
        ));
    }
    runtime
        .execute_submission(starts[2].clone(), submission(22))
        .unwrap();
    for (index, steer) in steer_commands.iter().enumerate() {
        runtime
            .execute_submission(steer.clone(), submission(23 + index as u8))
            .unwrap();
    }
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    let checkpoint_index = runtime
        .journal()
        .entries()
        .iter()
        .position(|entry| matches!(entry.record(), SemanticRecord::ContextCheckpoint(_)))
        .expect("runtime did not durably commit the proposed checkpoint");
    {
        let checkpoint_entries = runtime.journal().entries();
        let SemanticRecord::ContextCheckpoint(checkpoint) =
            checkpoint_entries[checkpoint_index].record()
        else {
            unreachable!()
        };
        assert_eq!(checkpoint.artifact_receipts().len(), 1);
        let final_steer_sequence = checkpoint_entries
            .iter()
            .find(|entry| {
                matches!(
                    entry.record(),
                    SemanticRecord::CommandCommitted(command)
                        if command.command() == &steer_commands[1]
                )
            })
            .expect("the second correction is durably recorded")
            .sequence();
        assert_eq!(checkpoint.source_journal_boundary(), final_steer_sequence);
        assert_eq!(
            checkpoint.retained_groups().last().unwrap().items(),
            active_group.as_slice()
        );
    }
    let observation = transcript
        .read_after(None)
        .into_entries()
        .into_iter()
        .find_map(|entry| match entry.record() {
            crate::TranscriptRecord::ContextCheckpointCommitted(observation) => {
                Some(observation.clone())
            },
            _ => None,
        })
        .expect("the committed lossy boundary must be visible to operators");
    assert_eq!(observation.input_tokens_before(), 90);
    assert_eq!(observation.input_tokens_after(), 40);
    assert_eq!(observation.retained_group_count(), 2);
    assert_eq!(observation.artifact_receipt_count(), 1);
    assert_eq!(observation.visible_prefix_loss_count(), 1);
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    let accepted_index = runtime
        .journal()
        .entries()
        .iter()
        .rposition(|entry| matches!(entry.record(), SemanticRecord::BackendRequestAccepted(_)))
        .expect("runtime did not record the successor request");
    assert!(checkpoint_index < accepted_index);
    assert!(matches!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(_)
    ));
    let entries = runtime.journal().entries();
    let SemanticRecord::ModelReplayDelta(delta) = entries
        .iter()
        .rev()
        .find(|entry| matches!(entry.record(), SemanticRecord::ModelReplayDelta(_)))
        .unwrap()
        .record()
    else {
        unreachable!()
    };
    assert_eq!(delta.context_epoch(), Some(2));
    assert_eq!(delta.delta().items().len(), 1);
    runtime.shutdown().unwrap();
}

// 도구 call/result Activity가 모두 닫힌 뒤 backend가 제출한 active suffix만 checkpoint의
// inline retained replay가 될 수 있고, 그 뒤 요청은 successor context epoch에 기록됩니다.
#[test]
fn binds_a_completed_tool_suffix_before_committing_the_mid_turn_checkpoint() {
    let session_id = session(9);
    let first_turn = turn(session_id, 1);
    let active_turn = turn(session_id, 2);
    let first_start = AgentCommand::StartTurn {
        turn: first_turn,
        input: UserInput::new("first input"),
    };
    let active_start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::new("inspect the workspace"),
    };
    let contract = ModelReplayContract::new("system", Vec::new());
    let first_group = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "first input".to_owned(),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "first answer".to_owned(),
            refusal: None,
        },
    ];
    let active_group = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "inspect the workspace".to_owned(),
            refusal: None,
        },
        ModelReplayItem::FunctionCall {
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        ModelReplayItem::FunctionCallOutput {
            call_id: "call-1".to_owned(),
            output: "workspace contents".to_owned(),
        },
    ];
    let policy = ContextPolicyChanged::try_new(
        1,
        true,
        ContextStrategy::PortableSummaryV1Alpha1,
        85,
        90,
        Some(10),
        Some(65_536),
    )
    .unwrap();
    let proposal = ContextCheckpointProposal::new(
        Some(active_turn),
        1,
        100,
        90,
        40,
        contract.clone(),
        "# Context Checkpoint\n\
## Current Objective\nContinue.\n\
## Active Constraints\nNone.\n\
## Decisions\nRetain the completed tool result.\n\
## Verified Progress\nThe prior turn completed.\n\
## Current State\nThe tool result is available.\n\
## Unknown or Unverified\nNone.\n\
## Next Actions\nContinue after the tool result.\n\
## Critical References\nREADME.md.",
        vec![first_group.clone()],
        Vec::new(),
        active_group.clone(),
        serde_json::json!({
            "schema": "yo.model-usage-receipt/v1",
            "response_id": "summary-tool",
            "round": 2,
            "provider": "test",
            "account": "default",
            "model": "test-model",
            "connector": "openai-responses",
            "api_dialect": "openai-responses",
            "base_url": "https://example.invalid/",
            "usage": {
                "input_tokens": 20,
                "output_tokens": 10,
                "total_tokens": 30,
                "reasoning_tokens": 0
            },
            "cache_read_input_tokens": { "availability": "unsupported" }
        }),
    )
    .unwrap();
    let call_activity = activity(active_turn, 1);
    let result_activity = activity(active_turn, 2);
    let final_delta = ModelReplayDelta::new(
        None,
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "final answer".to_owned(),
            refusal: None,
        }],
    );
    let create = AgentCommand::CreateSession { session_id };
    let steps = [
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(exact_replay_binding_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ContextPolicyChanged { policy }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: first_start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: first_turn,
            evidence: BackendOutcomeEvidence::without_identity()
                .with_replay(ModelReplayDelta::new(Some(contract), first_group)),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: active_start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: call_activity,
            kind: ActivityKind::ToolCall,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: call_activity,
            update: ActivityUpdate::TextSnapshot("read_file".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: call_activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: result_activity,
            kind: ActivityKind::ToolResult,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: result_activity,
            update: ActivityUpdate::TextSnapshot("workspace contents".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: result_activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ContextActiveSuffixCompleted {
            turn: active_turn,
            items: active_group,
        }),
        BackendScriptStep::Emit(BackendEvent::ContextCheckpointPrepared { proposal }),
        BackendScriptStep::Emit(BackendEvent::ModelRequestAccepted {
            turn: active_turn,
            evidence: request_evidence(),
        }),
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: active_turn,
            evidence: BackendOutcomeEvidence::without_identity().with_replay(final_delta),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let mut runtime = AgentRuntime::new(ScriptedBackend::new(steps));

    runtime.execute_command(create).unwrap();
    runtime
        .execute_submission(first_start, submission(30))
        .unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(crate::AgentEvent::TurnFinished { .. })
    ));
    runtime
        .execute_submission(active_start, submission(31))
        .unwrap();
    for _ in 0..6 {
        assert!(matches!(
            runtime.poll_event().unwrap(),
            crate::RuntimePoll::Event(_)
        ));
    }
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    let checkpoint_index = runtime
        .journal()
        .entries()
        .iter()
        .position(|entry| matches!(entry.record(), SemanticRecord::ContextCheckpoint(_)))
        .expect("mid-Turn checkpoint was not committed");
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    let accepted_index = runtime
        .journal()
        .entries()
        .iter()
        .rposition(|entry| matches!(entry.record(), SemanticRecord::BackendRequestAccepted(_)))
        .expect("successor request was not accepted");
    assert!(checkpoint_index < accepted_index);
    let accepted_operations = runtime
        .journal()
        .entries()
        .iter()
        .filter_map(|entry| match entry.record() {
            SemanticRecord::BackendRequestAccepted(accepted) => Some(accepted.operation_id()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(accepted_operations.len(), 3);
    assert_ne!(accepted_operations[1], accepted_operations[2]);
    assert!(matches!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(crate::AgentEvent::TurnFinished { .. })
    ));
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

fn exact_replay_binding_evidence() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "scripted",
        "1",
        BackendIdentity::new("scripted/binding/v1", "binding-1"),
        BackendIdentity::new("scripted/model/v1", "model-1"),
        BackendIdentity::new("scripted/session/v1", "session-1"),
        crate::ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: ReplayProfile::SemanticOnly,
        },
    )
}

fn request_evidence() -> BackendRequestEvidence {
    BackendRequestEvidence::new(
        "scripted/request/v1",
        BackendIdentity::new("scripted/exchange/v1", "exchange-1"),
        BackendIdentity::new("scripted/request/v1", "request-1"),
    )
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PreparedCommandState {
    prepared: bool,
    arm_count: usize,
    abort_count: usize,
    active_turn: Option<crate::TurnRef>,
    report_completion: bool,
}

struct PreparedSteeringBackend {
    state: Arc<Mutex<PreparedCommandState>>,
    misreport_start: bool,
}

impl BackendAdapter for PreparedSteeringBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        BackendStopHandle::no_op()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none().with_steer()
    }

    fn execute_command(
        &mut self,
        command: Self::Command,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        match command {
            AgentCommand::CreateSession { .. } => {
                Ok(BackendCommandEvidence::BindingOpened(binding_evidence()))
            },
            AgentCommand::StartTurn { turn, .. } if self.misreport_start => {
                let mut state = self.state.lock().unwrap();
                state.prepared = true;
                state.active_turn = Some(turn);
                Ok(BackendCommandEvidence::SubmissionPrepared)
            },
            AgentCommand::StartTurn { turn, .. } => {
                self.state.lock().unwrap().active_turn = Some(turn);
                Ok(BackendCommandEvidence::RequestAccepted(request_evidence()))
            },
            AgentCommand::SteerTurn { .. } => {
                self.state.lock().unwrap().prepared = true;
                Ok(BackendCommandEvidence::SubmissionPrepared)
            },
            _ => Ok(BackendCommandEvidence::None),
        }
    }

    fn commit_prepared_command(&mut self) -> Result<(), BackendFailure> {
        let mut state = self.state.lock().unwrap();
        if !state.prepared {
            return Err(BackendFailure::new(
                BackendFailureKind::Protocol,
                "no prepared steering input is available",
            ));
        }
        state.prepared = false;
        state.arm_count += 1;
        state.report_completion = true;
        Ok(())
    }

    fn abort_prepared_command(&mut self) -> Result<(), BackendFailure> {
        let mut state = self.state.lock().unwrap();
        if state.prepared {
            state.prepared = false;
            state.abort_count += 1;
        }
        Ok(())
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        let mut state = self.state.lock().unwrap();
        if state.report_completion {
            state.report_completion = false;
            return Ok(BackendPoll::Event(BackendEvent::ResumableTurnFinished {
                turn: state
                    .active_turn
                    .expect("an armed steer follows an active Turn"),
                evidence: BackendOutcomeEvidence::without_identity(),
            }));
        }
        Ok(BackendPoll::Pending)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

#[derive(Default)]
struct RuntimeTestRepository {
    entries: Vec<RepositoryEntry>,
}

impl SessionRepository for RuntimeTestRepository {
    fn append(
        &mut self,
        _session_id: crate::SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let sequence = RepositorySequence::new(
            u64::try_from(self.entries.len()).expect("test repository fits u64") + 1,
        );
        self.entries.push(RepositoryEntry::new(sequence, record));
        Ok(AppendReceipt::new(sequence))
    }

    fn read_after(
        &self,
        _session_id: crate::SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

// 내구성 Journal이 없는 Runtime은 준비된 조향 입력을 대기열에 넣지 않고, 제출 기록도 남기지 않아야
// 합니다.
#[test]
fn aborts_prepared_steering_when_submission_publication_is_not_durable() {
    let session_id = session(20);
    let active_turn = turn(session_id, 1);
    let state = Arc::new(Mutex::new(PreparedCommandState::default()));
    let backend = PreparedSteeringBackend {
        state: Arc::clone(&state),
        misreport_start: false,
    };
    let mut runtime = AgentRuntime::with_journal(backend, SessionJournal::new());
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("initial"),
            },
            submission(20),
        )
        .unwrap();
    let before = runtime.journal().entries();

    let error = runtime
        .execute_submission(
            AgentCommand::SteerTurn {
                turn: active_turn,
                input: UserInput::from("correction"),
            },
            submission(21),
        )
        .expect_err("a volatile Journal cannot authorize a prepared steering input");

    assert!(matches!(
        error,
        RuntimeError::Backend { ref failure, .. }
            if failure.kind() == BackendFailureKind::Session
    ));
    assert_eq!(runtime.journal().entries(), before);
    assert_eq!(
        *state.lock().unwrap(),
        PreparedCommandState {
            prepared: false,
            arm_count: 0,
            abort_count: 1,
            active_turn: Some(active_turn),
            report_completion: false,
        }
    );
    runtime.shutdown().unwrap();
}

// SubmissionPrepared 증거는 StartTurn과 짝지을 수 없으며, 잘못된 쌍의 backend 준비 상태를 폐기해야
// 합니다.
#[test]
fn aborts_prepared_submission_evidence_for_a_start_command() {
    let session_id = session(21);
    let active_turn = turn(session_id, 1);
    let state = Arc::new(Mutex::new(PreparedCommandState::default()));
    let backend = PreparedSteeringBackend {
        state: Arc::clone(&state),
        misreport_start: true,
    };
    let mut runtime = AgentRuntime::with_journal(backend, SessionJournal::new());
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();

    let error = runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("initial"),
            },
            submission(22),
        )
        .expect_err("prepared steering evidence cannot accept StartTurn");

    assert!(matches!(
        error,
        RuntimeError::Backend { ref failure, .. }
            if failure.kind() == BackendFailureKind::Protocol
    ));
    assert_eq!(runtime.active_turn(), None);
    assert!(runtime.journal().entries().iter().all(|entry| {
        !matches!(
            entry.record(),
            SemanticRecord::CommandCommitted(command)
                if matches!(command.command(), AgentCommand::StartTurn { .. })
        )
    }));
    assert_eq!(
        *state.lock().unwrap(),
        PreparedCommandState {
            prepared: false,
            arm_count: 0,
            abort_count: 1,
            active_turn: Some(active_turn),
            report_completion: false,
        }
    );
    runtime.shutdown().unwrap();
}

// Runtime은 backend에 전달하기 전에 exact Turn 대상을 확인하므로 잘못된 target은 준비·기록 없이
// 거절됩니다.
#[test]
fn rejects_a_wrong_steering_target_before_backend_preparation() {
    let session_id = session(23);
    let active_turn = turn(session_id, 1);
    let other_turn = turn(session_id, 2);
    let state = Arc::new(Mutex::new(PreparedCommandState::default()));
    let backend = PreparedSteeringBackend {
        state: Arc::clone(&state),
        misreport_start: false,
    };
    let mut runtime = AgentRuntime::with_journal(backend, SessionJournal::new());
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("initial"),
            },
            submission(25),
        )
        .unwrap();
    let before = runtime.journal().entries();

    let error = runtime
        .execute_submission(
            AgentCommand::SteerTurn {
                turn: other_turn,
                input: UserInput::from("wrong target"),
            },
            submission(26),
        )
        .expect_err("the Runtime rejects steering a different Turn before backend preparation");

    assert!(matches!(error, RuntimeError::CommandRejected(_)));
    assert_eq!(runtime.journal().entries(), before);
    assert_eq!(
        *state.lock().unwrap(),
        PreparedCommandState {
            prepared: false,
            arm_count: 0,
            abort_count: 0,
            active_turn: Some(active_turn),
            report_completion: false,
        }
    );
    runtime.shutdown().unwrap();
}

// 준비된 조향 명령은 정확한 ID로 먼저 내구성 기록되고, 후속 실제 요청 증거 전까지 이전 request
// anchor를 무효화합니다.
#[test]
fn durably_records_prepared_steer_and_rejects_completion_on_the_older_request() {
    let session_id = session(22);
    let active_turn = turn(session_id, 1);
    let mut journal = SessionJournal::with_repository_and_descriptor(
        Box::new(RuntimeTestRepository::default()),
        crate::fixture_descriptor(session_id),
    );
    journal.initialize_durability();
    let state = Arc::new(Mutex::new(PreparedCommandState::default()));
    let backend = PreparedSteeringBackend {
        state: Arc::clone(&state),
        misreport_start: false,
    };
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("initial"),
            },
            submission(23),
        )
        .unwrap();
    let steer = AgentCommand::SteerTurn {
        turn: active_turn,
        input: UserInput::from("same-turn correction"),
    };

    runtime
        .execute_submission(steer.clone(), submission(24))
        .expect("the durable Journal authorizes the prepared correction");

    assert!(runtime.journal().entries().iter().any(|entry| {
        matches!(
            entry.record(),
            SemanticRecord::CommandCommitted(command)
                if command.command() == &steer && command.submission_id() == Some(submission(24))
        )
    }));
    assert_eq!(state.lock().unwrap().arm_count, 1);
    let error = runtime
        .poll_event()
        .expect_err("the earlier request cannot anchor a Turn after a committed steer");
    assert!(error.to_string().contains("without an accepted request"));
    assert!(
        runtime
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.record(), SemanticRecord::ContinuationAnchor(_)))
    );
    runtime.shutdown().unwrap();
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct QuestionCommitProbe {
    fail_append: bool,
    fail_commit: bool,
    prepared: bool,
    commands: usize,
    polls: usize,
    commits: usize,
    aborts: usize,
    shutdowns: usize,
    trace: Vec<&'static str>,
}

struct QuestionRepository {
    inner: RuntimeTestRepository,
    probe: Arc<Mutex<QuestionCommitProbe>>,
}

impl SessionRepository for QuestionRepository {
    fn append(
        &mut self,
        session: crate::SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let mut probe = self.probe.lock().unwrap();
        if probe.prepared {
            if probe.fail_append {
                probe.trace.push("append failed");
                return Err(AppendError::Repository(RepositoryError::Unavailable {
                    message: "injected question append failure".into(),
                }));
            }
            probe.trace.push("appended");
        }
        self.inner.append(session, record)
    }

    fn read_after(
        &self,
        session: crate::SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        self.inner.read_after(session, sequence, limit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuestionBoundary {
    Complete,
    CapabilityAbsent,
    Secret,
    RequestIncomplete,
    ResponseIncomplete,
    WrongResponse,
    MalformedResult,
    ExtraResultField,
    AnsweredResult,
}

struct PreparedQuestionBackend {
    probe: Arc<Mutex<QuestionCommitProbe>>,
    events: VecDeque<BackendEvent>,
    active_turn: Option<crate::TurnRef>,
    boundary: QuestionBoundary,
}

impl PreparedQuestionBackend {
    fn new(probe: Arc<Mutex<QuestionCommitProbe>>) -> Self {
        Self {
            probe,
            events: VecDeque::new(),
            active_turn: None,
            boundary: QuestionBoundary::Complete,
        }
    }
}

impl BackendAdapter for PreparedQuestionBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

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
        self.probe.lock().unwrap().commands += 1;
        match command {
            AgentCommand::CreateSession { .. } => {
                self.events.push_back(BackendEvent::ContextPolicyChanged {
                    policy: ContextPolicyChanged::try_new(
                        1,
                        true,
                        ContextStrategy::PortableSummaryV1Alpha1,
                        85,
                        90,
                        Some(10),
                        Some(65_536),
                    )
                    .unwrap(),
                });
                Ok(BackendCommandEvidence::BindingOpened(
                    exact_replay_binding_evidence(),
                ))
            },
            AgentCommand::StartTurn { turn, .. } => {
                self.active_turn = Some(turn);
                let activity = activity(turn, 1);
                let question = crate::ActivityQuestion {
                    plain_text: "Question?".into(),
                    choices: Vec::new(),
                    allow_notes: false,
                    allow_unanswered: !matches!(
                        self.boundary,
                        QuestionBoundary::CapabilityAbsent | QuestionBoundary::Secret
                    ),
                    is_secret: self.boundary == QuestionBoundary::Secret,
                    storage_offer: None,
                    previous_question: false,
                    draft: None,
                    draft_choice: None,
                };
                self.events.extend([
                    BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::UserInputRequest {
                            request_id: RequestId::new(super::id(1)),
                        },
                    },
                    BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(question.to_snapshot().unwrap()),
                    },
                ]);
                Ok(BackendCommandEvidence::RequestAccepted(request_evidence()))
            },
            AgentCommand::RespondToActivity {
                response: ActivityResponse::QuestionUnanswered,
                ..
            } => {
                let mut probe = self.probe.lock().unwrap();
                probe.prepared = true;
                probe.trace.push("prepared");
                Ok(BackendCommandEvidence::OrdinaryQuestionResponsePrepared)
            },
            _ => Ok(BackendCommandEvidence::None),
        }
    }

    fn commit_prepared_command(&mut self) -> Result<(), BackendFailure> {
        let mut probe = self.probe.lock().unwrap();
        assert!(probe.prepared);
        probe.commits += 1;
        probe.trace.push("committed");
        if probe.fail_commit {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "injected commit failure",
            ));
        }
        probe.prepared = false;
        let turn = self.active_turn.unwrap();
        let request_activity = activity(turn, 1);
        if self.boundary != QuestionBoundary::RequestIncomplete {
            self.events.push_back(BackendEvent::ActivityFinished {
                activity: request_activity,
                outcome: ActivityOutcome::Completed,
            });
        }
        let activity = activity(turn, 2);
        self.events.extend([
            BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputResponse {
                    request_id: RequestId::new(super::id(
                        if self.boundary == QuestionBoundary::WrongResponse {
                            2
                        } else {
                            1
                        },
                    )),
                },
            },
            BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot("No answer provided.".into()),
            },
        ]);
        if self.boundary != QuestionBoundary::ResponseIncomplete {
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        }
        self.events.extend([
            BackendEvent::ContextActiveSuffixCompleted {
                turn,
                items: vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: "question task".into(),
                        refusal: None,
                    },
                    ModelReplayItem::FunctionCall {
                        call_id: "question-call".into(),
                        name: "ask_user".into(),
                        arguments: r#"{"title":"Question","question":"Question?"}"#.into(),
                    },
                    ModelReplayItem::FunctionCallOutput {
                        call_id: "question-call".into(),
                        output: match self.boundary {
                            QuestionBoundary::MalformedResult => "not json",
                            QuestionBoundary::ExtraResultField => r#"{"schema":"yo.ask-user-result/v1","status":"unanswered","text":null}"#,
                            QuestionBoundary::AnsweredResult => r#"{"schema":"yo.ask-user-result/v1","status":"answered","kind":"text","text":"invented"}"#,
                            _ => r#"{"schema":"yo.ask-user-result/v1","status":"unanswered"}"#,
                        }.into(),
                    },
                ],
            },
            BackendEvent::TurnFinished {
                turn,
                outcome: TurnOutcome::Completed,
            },
        ]);
        Ok(())
    }

    fn abort_prepared_command(&mut self) -> Result<(), BackendFailure> {
        let mut probe = self.probe.lock().unwrap();
        assert!(probe.prepared);
        probe.prepared = false;
        probe.aborts += 1;
        probe.trace.push("aborted");
        Ok(())
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.probe.lock().unwrap().polls += 1;
        Ok(self
            .events
            .pop_front()
            .map_or(BackendPoll::Pending, BackendPoll::Event))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.probe.lock().unwrap().shutdowns += 1;
        Ok(())
    }
}

fn runtime_with_prepared_question(
    boundary: QuestionBoundary,
) -> (
    AgentRuntime<Box<dyn crate::AgentBackend + Send>>,
    Arc<Mutex<QuestionCommitProbe>>,
    ActivityRequestRef,
) {
    let session_id = session(60);
    let turn = turn(session_id, 1);
    let probe = Arc::new(Mutex::new(QuestionCommitProbe::default()));
    let mut journal = SessionJournal::with_repository_and_descriptor(
        Box::new(QuestionRepository {
            inner: RuntimeTestRepository::default(),
            probe: Arc::clone(&probe),
        }),
        crate::fixture_descriptor(session_id),
    );
    journal.initialize_durability();
    let mut backend = PreparedQuestionBackend::new(Arc::clone(&probe));
    backend.boundary = boundary;
    let backend: Box<dyn crate::AgentBackend + Send> = Box::new(backend);
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn,
                input: UserInput::new("question task"),
            },
            submission(60),
        )
        .unwrap();
    for _ in 0..2 {
        assert!(matches!(
            runtime.poll_event().unwrap(),
            crate::RuntimePoll::Event(_)
        ));
    }
    let request = ActivityRequestRef::new(activity(turn, 1), RequestId::new(super::id(1)));
    (runtime, probe, request)
}

// 실제 repository append 실패 또는 후속 backend commit 실패 뒤에는 공개 실행 API,
// poll 및 replacement가 backend를 다시 호출하지 않고 준비 응답을 정확히 한 번 폐기합니다.
#[test]
fn question_unanswered_failed_commit_blocks_every_continuation_entrypoint() {
    for fail_backend_commit in [false, true] {
        let (mut runtime, probe, request) =
            runtime_with_prepared_question(QuestionBoundary::Complete);
        {
            let mut state = probe.lock().unwrap();
            state.fail_append = !fail_backend_commit;
            state.fail_commit = fail_backend_commit;
        }
        let response = AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::QuestionUnanswered,
        };
        assert!(
            runtime
                .execute_command(response.clone())
                .unwrap_err()
                .to_string()
                .contains("ordinary question response")
        );
        let after_failure = probe.lock().unwrap().clone();
        assert_eq!(after_failure.commits, usize::from(fail_backend_commit));
        assert_eq!(after_failure.aborts, 1);
        assert!(!after_failure.prepared);
        assert_eq!(
            after_failure.trace,
            if fail_backend_commit {
                vec!["prepared", "appended", "committed", "aborted"]
            } else {
                vec!["prepared", "append failed", "aborted"]
            }
        );
        assert_eq!(runtime.journal().entries().iter().any(|entry| matches!(entry.record(), SemanticRecord::CommandCommitted(command) if command.command() == &response)), fail_backend_commit);
        for error in [
            runtime
                .execute_command(AgentCommand::InterruptTurn {
                    turn: request.activity().turn(),
                })
                .unwrap_err(),
            runtime.execute_command(response).unwrap_err(),
            runtime
                .execute_submission(
                    AgentCommand::SteerTurn {
                        turn: request.activity().turn(),
                        input: UserInput::new("continue"),
                    },
                    submission(61),
                )
                .unwrap_err(),
            runtime.poll_event().unwrap_err(),
        ] {
            assert!(
                error.to_string().contains("continuation is blocked"),
                "{error}"
            );
        }
        let candidate_probe = Arc::new(Mutex::new(QuestionCommitProbe::default()));
        let candidate = Box::new(PreparedQuestionBackend::new(Arc::clone(&candidate_probe)));
        let replacement = runtime
            .replace_backend(candidate)
            .expect_err("replacement must be blocked");
        assert!(
            replacement
                .primary
                .to_string()
                .contains("continuation is blocked")
        );
        assert_eq!(
            *candidate_probe.lock().unwrap(),
            QuestionCommitProbe {
                shutdowns: 1,
                ..QuestionCommitProbe::default()
            }
        );
        assert_eq!(*probe.lock().unwrap(), after_failure);
        runtime.shutdown().unwrap();
        assert_eq!(probe.lock().unwrap().shutdowns, 1);
    }
}

// ordinary 응답은 먼저 Journal에 기록된 뒤 backend를 열며 비밀 terminal latch를 설정하지
// 않습니다. 질문/응답만 있는 완료 group도 compaction 경계로 인정하고 다음 Turn을 허용합니다.
#[test]
fn question_unanswered_durable_commit_precedes_continuation_without_terminalizing_session() {
    let (mut runtime, probe, request) = runtime_with_prepared_question(QuestionBoundary::Complete);
    let before_polls = probe.lock().unwrap().polls;
    let response = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::QuestionUnanswered,
    };
    runtime.execute_command(response.clone()).unwrap();
    {
        let state = probe.lock().unwrap();
        assert_eq!(state.trace, ["prepared", "appended", "committed"]);
        assert_eq!(state.commits, 1);
        assert_eq!(state.aborts, 0);
        assert_eq!(state.polls, before_polls);
    }
    assert_eq!(runtime.journal().entries().iter().filter(|entry| matches!(entry.record(), SemanticRecord::CommandCommitted(command) if command.command() == &response)).count(), 1);
    for _ in 0..4 {
        assert!(matches!(
            runtime.poll_event().unwrap(),
            crate::RuntimePoll::Event(_)
        ));
    }
    assert_eq!(runtime.poll_event().unwrap(), crate::RuntimePoll::Pending);
    assert!(matches!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(crate::AgentEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        })
    ));
    let next = turn(request.activity().turn().session_id(), 2);
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: next,
                input: UserInput::new("next task"),
            },
            submission(62),
        )
        .unwrap();
    assert_eq!(runtime.active_turn(), Some(next));
    runtime.shutdown().unwrap();
}

// 잘못된 capability, 비밀 요청, 미완료 Activity 또는 다른 request의 응답은
// 질문 call/result가 있어도 실제 runtime의 active compaction 경계를 통과하지 못합니다.
#[test]
fn question_unanswered_active_suffix_rejects_untrusted_question_boundaries() {
    for boundary in [
        QuestionBoundary::CapabilityAbsent,
        QuestionBoundary::Secret,
        QuestionBoundary::RequestIncomplete,
        QuestionBoundary::ResponseIncomplete,
        QuestionBoundary::WrongResponse,
        QuestionBoundary::MalformedResult,
        QuestionBoundary::ExtraResultField,
        QuestionBoundary::AnsweredResult,
    ] {
        let (mut runtime, _, request) = runtime_with_prepared_question(boundary);
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::QuestionUnanswered,
            })
            .unwrap();
        let mut rejected = false;
        for _ in 0..6 {
            match runtime.poll_event() {
                Err(_) => {
                    rejected = true;
                    break;
                },
                Ok(
                    crate::RuntimePoll::Event(crate::AgentEvent::TurnFinished { .. })
                    | crate::RuntimePoll::Pending
                    | crate::RuntimePoll::Closed,
                ) => panic!("untrusted {boundary:?} crossed its semantic boundary"),
                Ok(crate::RuntimePoll::Event(_)) => {},
            }
        }
        assert!(rejected, "{boundary:?} must reject before continuation");
        runtime.shutdown().unwrap();
    }
}

mod local_failure;
