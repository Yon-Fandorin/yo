use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, BackendEvent, BackendFailureKind, BackendPoll, ModelConnectorEvent,
    ToolApprovalRequirement, TurnOutcome, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, MockConnector, MockHost, binding, drain_until_turn, event_rounds, registry,
        turn,
    },
    fixtures::{
        SequenceTokenCounter, completed_summary_round, completed_text_round, portable_summary,
        turn_number,
    },
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 추론 뒤 첫 텍스트 메시지를 허용해도 다른 메시지·완료 identity·추가 content나
// 완료 뒤 텍스트는 자동/수동 압축 모두 checkpoint 없이 fail-closed 처리한다.
#[test]
fn compaction_rejects_mismatched_or_extra_summary_message_events() {
    let delta = |output_index, item_id: &str, content_index| ModelConnectorEvent::TextDelta {
        output_index,
        item_id: item_id.to_owned(),
        content_index,
        delta: "must not enter a checkpoint".to_owned(),
    };
    let done = |output_index, item_id: &str| ModelConnectorEvent::MessageDone {
        output_index,
        item_id: item_id.to_owned(),
    };
    let cases = [
        ("second output slot", vec![delta(2, "invalid-item", 0)]),
        ("second item identity", vec![delta(1, "other-item", 0)]),
        ("second content part", vec![delta(1, "invalid-item", 1)]),
        ("mismatched done slot", vec![done(2, "invalid-item")]),
        ("mismatched done identity", vec![done(1, "other-item")]),
        (
            "duplicate completion",
            vec![done(1, "invalid-item"), done(1, "invalid-item")],
        ),
        (
            "text after completion",
            vec![done(1, "invalid-item"), delta(1, "invalid-item", 0)],
        ),
    ];
    for idle in [false, true] {
        for (name, invalid_events) in &cases {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let mut summary_round = completed_summary_round("invalid", &portable_summary(), 1);
            summary_round.splice(3..3, invalid_events.clone());
            let mut backend = NativeModelBackend::with_connector(
                Box::new(MockConnector {
                    rounds: event_rounds(vec![
                        completed_text_round("one", "first"),
                        completed_text_round("two", "second"),
                        summary_round,
                    ]),
                    requests: Arc::clone(&requests),
                }),
                binding(),
                registry(ToolApprovalRequirement::Automatic),
                NativeModelBackendServices::new(
                    Box::new(yo_core::admit_standard_complete_binding),
                    Some(Box::new(ExactAdmission)),
                    Box::new(MockHost::default()),
                    Box::new(SequenceTokenCounter::new(
                        [10, 10, if idle { 70 } else { 90 }, 20],
                        Arc::new(Mutex::new(Vec::new())),
                    )),
                ),
                yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
                NativeModelBackendConfig::default(),
            )
            .unwrap();
            backend
                .execute_command(AgentCommand::CreateSession {
                    session_id: turn().session_id(),
                })
                .unwrap();
            for number in [1, 2] {
                backend
                    .execute_command(AgentCommand::StartTurn {
                        turn: turn_number(number),
                        input: UserInput::from(format!("input-{number}")),
                    })
                    .unwrap();
                assert!(matches!(
                    drain_until_turn(&mut backend),
                    BackendEvent::ResumableTurnFinished { .. }
                ));
            }
            let original_replay = backend.replay.clone();
            if idle {
                backend
                    .execute_command(AgentCommand::CompactContext { guidance: None })
                    .unwrap();
            } else {
                backend
                    .execute_command(AgentCommand::StartTurn {
                        turn: turn_number(3),
                        input: UserInput::from("input-3"),
                    })
                    .unwrap();
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "{name}, idle={idle}: missing failure"
                );
                match backend.poll_event() {
                    Err(error) => {
                        assert!(idle, "automatic compaction must finish its Turn");
                        assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
                        break;
                    },
                    Ok(BackendPoll::Event(BackendEvent::TurnFinished {
                        outcome: TurnOutcome::Failed(error),
                        ..
                    })) => {
                        assert!(!idle);
                        assert_eq!(error.code(), Some("context_exhausted"));
                        break;
                    },
                    Ok(BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { .. })) => {
                        panic!("{name}, idle={idle}: invalid checkpoint")
                    },
                    Ok(BackendPoll::Closed) => panic!("{name}, idle={idle}: missing typed failure"),
                    _ => thread::yield_now(),
                }
            }
            assert_eq!(requests.lock().unwrap().len(), 3);
            assert_eq!(backend.replay, original_replay);
            assert!(backend.context_exhausted);
            let error = backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn_number(4),
                    input: UserInput::from("must not dispatch"),
                })
                .unwrap_err();
            assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
            assert_eq!(requests.lock().unwrap().len(), 3);
            backend.shutdown().unwrap();
        }
    }
}
