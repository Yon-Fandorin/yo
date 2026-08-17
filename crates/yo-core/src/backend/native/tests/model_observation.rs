use std::sync::{Arc, Mutex};

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
use crate::{ModelRequestFailureKind, ModelRequestOutcome, UserInput};

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
            usage: crate::ResponsesUsage::default(),
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
                usage: crate::ResponsesUsage::default(),
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
            update: crate::ActivityUpdate::TextSnapshot(text),
            ..
        } if text.contains("model request status was not saved")
            && text.contains("repository-contention")
    )));
    assert!(matches!(
        events.last(),
        Some(BackendEvent::ResumableTurnFinished { .. })
    ));
}
