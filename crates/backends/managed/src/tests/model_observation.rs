use std::sync::{Arc, Mutex};

use serde_json::json;
use yo_core::{
    CacheReadInputTokens, ModelRequestFailureKind, ModelRequestOutcome, ResponsesUsage, UserInput,
    VersionedProfileId,
};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendEvent, BackendPoll, ModelConnectorEvent,
        ModelConnectorTerminal, NativeModelBackend, NativeModelBackendConfig,
        NativeModelBackendServices, ToolApprovalRequirement,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, context_profile,
        event_rounds, registry, turn,
    },
};

fn backend(
    rounds: Vec<Vec<ModelConnectorEvent>>,
    outcomes: Arc<Mutex<Vec<ModelRequestOutcome>>>,
    observer_failure: Option<&'static str>,
) -> NativeModelBackend {
    let services = NativeModelBackendServices::new(
        Some(Box::new(ExactAdmission)),
        Box::new(MockHost::default()),
        Box::new(FixedTokenCounter(1)),
    )
    .with_model_request_observer(move |outcome| {
        outcomes.lock().unwrap().push(outcome);
        observer_failure.map_or(Ok(()), |message| Err(message.to_owned()))
    });
    NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        services,
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap()
}

fn successful_round() -> Vec<ModelConnectorEvent> {
    successful_round_with_usage(ResponsesUsage::default())
}

fn successful_round_with_usage(usage: ResponsesUsage) -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "response".to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: "message".to_owned(),
            content_index: 0,
            delta: "done".to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: "message".to_owned(),
        },
        ModelConnectorEvent::Terminal {
            response_id: "response".to_owned(),
            status: ModelConnectorTerminal::Completed,
            usage,
        },
    ]
}

fn run_turn(backend: &mut NativeModelBackend) -> Vec<BackendEvent> {
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("hello"),
        })
        .unwrap();
    let mut events = Vec::new();
    for _ in 0..100 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event) => {
                let terminal = matches!(
                    event,
                    BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
                );
                events.push(event);
                if terminal {
                    return events;
                }
            },
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the Turn outcome"),
        }
    }
    panic!("backend did not finish within the deterministic poll budget")
}

// connector의 완전한 terminal success와 typed incomplete failure는 provider body를 전달하지
// 않고 각각 성공과 connector가 확정한 closed outcome 하나로 host observer에 보고합니다.
#[test]
fn terminal_outcomes_are_reported_once_with_closed_classification() {
    let succeeded = Arc::new(Mutex::new(Vec::new()));
    run_turn(&mut backend(
        vec![successful_round()],
        succeeded.clone(),
        None,
    ));
    assert_eq!(*succeeded.lock().unwrap(), [ModelRequestOutcome::Succeeded]);

    let rejected = Arc::new(Mutex::new(Vec::new()));
    run_turn(&mut backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "response".to_owned(),
            },
            ModelConnectorEvent::Terminal {
                response_id: "response".to_owned(),
                status: ModelConnectorTerminal::Incomplete {
                    reason: Some("provider-private-sentinel".to_owned()),
                    request_failure: ModelRequestFailureKind::ProviderUnavailable,
                },
                usage: yo_core::ResponsesUsage::default(),
            },
        ]],
        rejected.clone(),
        None,
    ));
    assert_eq!(
        *rejected.lock().unwrap(),
        [ModelRequestOutcome::Failed(
            ModelRequestFailureKind::ProviderUnavailable
        )]
    );
}

// 완료된 response 하나는 귀속과 token 수를 가진 versioned usage receipt 하나만 만들고,
// reported 0·absent·unsupported cache-read 상태를 서로 다른 닫힌 JSON shape로 보존합니다.
#[test]
fn terminal_usage_receipt_preserves_closed_cache_read_availability() {
    let profile = || VersionedProfileId::new("kimi.usage.cached-tokens/v1").unwrap();
    for (cache_read_input_tokens, expected_cache) in [
        (
            CacheReadInputTokens::Reported {
                tokens: 0,
                source_profile: profile(),
            },
            json!({
                "availability": "reported",
                "tokens": 0,
                "source_profile": "kimi.usage.cached-tokens/v1",
            }),
        ),
        (
            CacheReadInputTokens::Absent {
                source_profile: profile(),
            },
            json!({
                "availability": "absent",
                "source_profile": "kimi.usage.cached-tokens/v1",
            }),
        ),
        (
            CacheReadInputTokens::Unsupported,
            json!({"availability": "unsupported"}),
        ),
    ] {
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let events = run_turn(&mut backend(
            vec![successful_round_with_usage(ResponsesUsage {
                input_tokens: Some(12),
                output_tokens: Some(7),
                total_tokens: Some(19),
                reasoning_tokens: Some(3),
                cache_read_input_tokens,
            })],
            outcomes,
            None,
        ));
        let receipts = events
            .iter()
            .filter_map(|event| match event {
                BackendEvent::ActivityUpdated {
                    update: yo_core::ActivityUpdate::TextSnapshot(text),
                    ..
                } => serde_json::from_str::<serde_json::Value>(text).ok(),
                _ => None,
            })
            .filter(|value| value["schema"] == "yo.model-usage-receipt/v1")
            .collect::<Vec<_>>();

        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0]["response_id"], "response");
        assert_eq!(receipts[0]["round"], 1);
        assert_eq!(receipts[0]["provider"], "qwencloud");
        assert_eq!(receipts[0]["account"], "default");
        assert_eq!(receipts[0]["model"], "qwen3.8max");
        assert_eq!(receipts[0]["connector"], "openai-responses");
        assert_eq!(receipts[0]["api_dialect"], "openai-responses");
        assert_eq!(receipts[0]["base_url"], "https://example.invalid/v1");
        assert_eq!(
            receipts[0]["usage"],
            json!({
                "input_tokens": 12,
                "output_tokens": 7,
                "total_tokens": 19,
                "reasoning_tokens": 3,
            })
        );
        assert_eq!(receipts[0]["cache_read_input_tokens"], expected_cache);
        assert_eq!(receipts[0].as_object().unwrap().len(), 11);
    }
}

// terminal 없이 닫힌 stream은 protocol failure로 기록하지만 사용자 interrupt는 관찰을
// 만들지 않아 cancellation이나 cleanup을 provider failure로 오분류하지 않습니다.
#[test]
fn protocol_close_is_observed_but_user_cancellation_is_not() {
    let protocol = Arc::new(Mutex::new(Vec::new()));
    run_turn(&mut backend(vec![Vec::new()], protocol.clone(), None));
    assert_eq!(
        *protocol.lock().unwrap(),
        [ModelRequestOutcome::Failed(
            ModelRequestFailureKind::Protocol
        )]
    );

    let cancelled = Arc::new(Mutex::new(Vec::new()));
    let mut backend = backend(vec![Vec::new()], cancelled.clone(), None);
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("hello"),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::InterruptTurn { turn: turn() })
        .unwrap();
    assert!(cancelled.lock().unwrap().is_empty());
}

// observation 저장 실패는 원래 성공 outcome을 바꾸지 않고 별도 model-work warning
// activity로 전달되어 사용자가 저장 실패와 provider 결과를 구분할 수 있습니다.
#[test]
fn persistence_failure_is_a_separate_warning_activity() {
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let events = run_turn(&mut backend(
        vec![successful_round()],
        outcomes.clone(),
        Some("repository-contention"),
    ));
    assert_eq!(*outcomes.lock().unwrap(), [ModelRequestOutcome::Succeeded]);
    assert!(events.iter().any(|event| matches!(
        event,
        BackendEvent::ActivityUpdated {
            update: yo_core::ActivityUpdate::TextSnapshot(text),
            ..
        } if text.contains("model request status was not saved")
            && text.contains("repository-contention")
    )));
    assert!(matches!(
        events.last(),
        Some(BackendEvent::ResumableTurnFinished { .. })
    ));
}
