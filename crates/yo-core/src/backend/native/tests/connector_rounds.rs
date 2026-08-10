use std::sync::{Arc, Mutex};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendEvent, EffectiveModelBinding, ModelConnectorEvent,
        ModelConnectorTerminal, ModelReplayItem, ModelReplayRole, NativeModelBackend,
        NativeModelBackendConfig, NativeModelBackendServices, ReasoningChannel,
        ToolApprovalRequirement, TurnOutcome,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, completed,
        context_profile, drain_until_turn, event_rounds, registry, turn,
    },
};
use crate::{AccountId, ApiDialect, ModelId, NormalizedEndpoint, ProviderId, UserInput};

fn chat_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://example.invalid/v1").unwrap(),
    )
}

// visible refusal은 실패로 바꾸지 않고 Chat 전용 response identity와 assistant replay를 함께
// 보존한다.
#[test]
fn native_backend_commits_visible_refusal_as_a_normal_assistant_message() {
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "chat-refusal".to_owned(),
                },
                ModelConnectorEvent::RefusalDelta {
                    output_index: 0,
                    item_id: "message".to_owned(),
                    content_index: 1,
                    delta: "요청을 처리할 수 없습니다".to_owned(),
                },
                ModelConnectorEvent::MessageDone {
                    output_index: 0,
                    item_id: "message".to_owned(),
                },
                completed("chat-refusal"),
            ]]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        chat_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("거절 테스트"),
        })
        .unwrap();

    let BackendEvent::ResumableTurnFinished { evidence, .. } = drain_until_turn(&mut backend)
    else {
        panic!("a visible refusal must complete resumably")
    };
    assert_eq!(
        evidence.model_replay().unwrap().items().last(),
        Some(&ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: String::new(),
            refusal: Some("요청을 처리할 수 없습니다".to_owned()),
        })
    );
    assert_eq!(
        evidence.outcome_identity().unwrap().schema(),
        "chat-completions.response-id/v1"
    );
}

// length로 끝난 partial Chat round는 실패 Turn이 되며 replay나 Anchor 후보를 만들지 않는다.
#[test]
fn incomplete_chat_round_fails_without_a_resumable_replay_delta() {
    let mut backend = backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "chat-length".to_owned(),
            },
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "partial".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            ModelConnectorEvent::Terminal {
                response_id: "chat-length".to_owned(),
                status: ModelConnectorTerminal::Incomplete {
                    reason: Some("length".to_owned()),
                },
                usage: crate::ModelConnectorUsage::default(),
            },
        ]],
        ToolApprovalRequirement::Automatic,
        Arc::new(Mutex::new(0)),
    );
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("truncate"),
        })
        .unwrap();

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("an incomplete Chat round must fail the Turn")
    };
    assert_eq!(failure.message(), "model response was incomplete: length");
}

// 빈 assistant message item은 의도적인 최종 응답으로 보존하지만 message item이 전혀 없는
// reasoning-only 완료는 재개 가능한 Turn으로 잘못 봉인하지 않는다.
#[test]
fn native_backend_requires_a_final_assistant_message_item() {
    let starts = Arc::new(Mutex::new(0));
    let mut empty_message = backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "empty".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("empty"),
        ]],
        ToolApprovalRequirement::Automatic,
        Arc::clone(&starts),
    );
    empty_message
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    empty_message
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();
    assert!(matches!(
        drain_until_turn(&mut empty_message),
        BackendEvent::ResumableTurnFinished { .. }
    ));

    let mut reasoning_only = backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "reasoning".to_owned(),
            },
            ModelConnectorEvent::ReasoningDelta {
                output_index: 0,
                item_id: "reason".to_owned(),
                channel: ReasoningChannel::Summary,
                part_index: 0,
                delta: "summary".to_owned(),
            },
            completed("reasoning"),
        ]],
        ToolApprovalRequirement::Automatic,
        starts,
    );
    reasoning_only
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    reasoning_only
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();
    assert!(matches!(
        drain_until_turn(&mut reasoning_only),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
}
