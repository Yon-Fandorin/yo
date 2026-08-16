use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendCommandEvidence, BackendEvent, BackendFailureKind,
        EffectiveModelBinding, ModelConnectorEvent, ModelConnectorTerminal, ModelReplayItem,
        ModelReplayRole, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
        ReasoningChannel, ToolApprovalRequirement, TurnOutcome,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, completed,
        context_profile, drain_until_turn, event_rounds, registry, turn,
    },
};
use crate::{
    AccountId, ApiDialect, KimiAssistantMessage, ModelContextProfile, ModelId, ModelProfileLayer,
    ModelProfileParameters, NormalizedEndpoint, ProviderId, ReplayProfile, UserInput,
    VersionedProfileId,
};

fn chat_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://example.invalid/v1").unwrap(),
    )
}

fn kimi_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        ModelId::new("kimi-k3").unwrap(),
        ApiDialect::KimiChatCompletions,
        NormalizedEndpoint::parse("https://api.moonshot.ai/v1").unwrap(),
    )
}

fn kimi_k27_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        ModelId::new("kimi-k2.7-code").unwrap(),
        ApiDialect::KimiChatCompletions,
        NormalizedEndpoint::parse("https://api.moonshot.ai/v1").unwrap(),
    )
}

fn kimi_profile() -> crate::EffectiveModelProfile {
    let layer = ModelProfileLayer::new(
        Some(ApiDialect::KimiChatCompletions),
        Some(VersionedProfileId::new("utf8-bytes/v1").unwrap()),
        Some(1_048_576),
        Some(131_072),
        Some(serde_json::from_str::<ModelProfileParameters>(r#"{"effort":"max"}"#).unwrap()),
        Some(serde_json::from_str::<ModelProfileParameters>("{}").unwrap()),
        Some(VersionedProfileId::new("local-tools/v1").unwrap()),
        Some(VersionedProfileId::new("semantic-terminal/v1").unwrap()),
    )
    .with_replay_profile(Some(
        VersionedProfileId::new("kimi-private-local-plaintext/v1").unwrap(),
    ));
    crate::EffectiveModelProfile::resolve(None, &layer).unwrap()
}

fn kimi_k27_profile() -> crate::EffectiveModelProfile {
    let layer = ModelProfileLayer::new(
        Some(ApiDialect::KimiChatCompletions),
        Some(VersionedProfileId::new("utf8-bytes/v1").unwrap()),
        Some(262_144),
        Some(32_768),
        Some(serde_json::from_str::<ModelProfileParameters>("{}").unwrap()),
        Some(
            serde_json::from_str::<ModelProfileParameters>(
                r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
            )
            .unwrap(),
        ),
        Some(VersionedProfileId::new("local-tools/v1").unwrap()),
        Some(VersionedProfileId::new("semantic-terminal/v1").unwrap()),
    )
    .with_replay_profile(Some(
        VersionedProfileId::new("kimi-private-local-plaintext/v1").unwrap(),
    ));
    crate::EffectiveModelProfile::resolve(None, &layer).unwrap()
}

fn kimi_round(response: &str, visible: &str, private: &str) -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: response.to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: "message".to_owned(),
            content_index: 0,
            delta: visible.to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: "message".to_owned(),
        },
        ModelConnectorEvent::ProviderPrivateAssistant {
            output_index: 1,
            schema: "kimi.assistant-message/v1alpha1".to_owned(),
            message: KimiAssistantMessage::new(private, Some(visible.to_owned()), Vec::new()),
        },
        completed(response),
    ]
}

struct ToolAwareBoundaryCounter {
    saw_tools: Arc<Mutex<bool>>,
}

impl crate::ModelTokenCounter for ToolAwareBoundaryCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        payload: &serde_json::Value,
    ) -> Result<u64, crate::ModelTokenCounterError> {
        let saw_tools = payload
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tools| !tools.is_empty());
        *self.saw_tools.lock().unwrap() = saw_tools;
        Ok(if saw_tools { 229_377 } else { 229_376 })
    }
}

// Kimi private profile은 binding evidence에 명시되고 한 round의 private assistant가 durable
// replay delta에 남아 다음 round의 connector input으로 실제 재전송되는지 판별합니다.
#[test]
fn native_backend_preserves_and_reuses_kimi_private_assistant_state() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                kimi_round("kimi-1", "first", "hidden-1"),
                kimi_round("kimi-2", "second", "hidden-2"),
            ]),
            requests: Arc::clone(&requests),
        }),
        kimi_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        ModelContextProfile::new(1_048_576, 131_072, "utf8-bytes/v1").unwrap(),
        Some(kimi_profile()),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    let opened = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    let BackendCommandEvidence::BindingOpened(opened) = opened else {
        panic!("Kimi Session must publish binding evidence")
    };
    assert!(matches!(
        opened.continuation_strategy(),
        crate::ContinuationStrategy::ExactReplay {
            replay_profile: ReplayProfile::KimiPrivateLocalPlaintext,
            ..
        }
    ));

    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("first request"),
        })
        .unwrap();
    let BackendEvent::ResumableTurnFinished { evidence, .. } = drain_until_turn(&mut backend)
    else {
        panic!("the first Kimi round must finish resumably")
    };
    assert!(
        evidence
            .model_replay()
            .unwrap()
            .items()
            .iter()
            .any(|item| matches!(
                item,
                ModelReplayItem::ProviderPrivateAssistant { message, .. }
                    if message.reasoning_content() == "hidden-1"
            ))
    );

    let second_turn = crate::TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second request"),
        })
        .unwrap();
    let _ = drain_until_turn(&mut backend);
    let requests = requests.lock().unwrap();
    assert!(requests[1].input().iter().any(|item| matches!(
        item,
        crate::ModelConnectorInputItem::ProviderPrivateAssistant { message, .. }
            if message.reasoning_content() == "hidden-1"
    )));
}

// backend는 Kimi private object 단독 크기가 아니라 같은 turn의 visible semantic item과
// contract를 합친 canonical replay delta를 저장 전에 재며 exact 16 MiB는 받고 +1은 버립니다.
#[test]
fn native_backend_bounds_complete_semantic_and_private_replay_before_retention() {
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        kimi_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        ModelContextProfile::new(1_048_576, 131_072, "utf8-bytes/v1").unwrap(),
        Some(kimi_profile()),
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
            input: UserInput::from("size the complete replay delta"),
        })
        .unwrap();

    let visible = "v".repeat(4 * 1024 * 1024);
    let mut state = backend.turn.take().unwrap();
    state.round_message_items.insert(0);
    state.round_messages.insert((0, 0), visible.clone());
    let item = |reasoning_bytes| ModelReplayItem::ProviderPrivateAssistant {
        schema: "kimi.assistant-message/v1alpha1".to_owned(),
        message: KimiAssistantMessage::new(
            "r".repeat(reasoning_bytes),
            Some(visible.clone()),
            Vec::new(),
        ),
    };

    let empty_private = item(0);
    let fixed_bytes = backend
        .prospective_replay_delta_encoded_len(&state, Some((1, &empty_private)))
        .unwrap();
    let accepted = super::super::ModelReplayDelta::MAX_ENCODED_BYTES - fixed_bytes;
    backend
        .ensure_replay_capacity_with_round_item(&state, Some((1, &item(accepted))))
        .expect("the exact complete replay boundary is admitted");

    let overflow = backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::ProviderPrivateAssistant {
                output_index: 1,
                schema: "kimi.assistant-message/v1alpha1".to_owned(),
                message: KimiAssistantMessage::new(
                    "r".repeat(accepted + 1),
                    Some(visible.clone()),
                    Vec::new(),
                ),
            },
        )
        .unwrap_err();
    assert_eq!(overflow.kind(), BackendFailureKind::ContextExhausted);
    assert!(state.round_replay.is_empty());

    backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::ProviderPrivateAssistant {
                output_index: 1,
                schema: "kimi.assistant-message/v1alpha1".to_owned(),
                message: KimiAssistantMessage::new("r".repeat(accepted), Some(visible), Vec::new()),
            },
        )
        .unwrap();
    assert!(state.round_replay.contains_key(&1));
}

// K2.7의 262,144 context에서 32,768 output을 예약한 뒤, message만이 아니라 connector가
// 만든 tool 포함 전체 payload가 229,377이면 equality 경계 229,376을 넘어 전송 전에 닫힙니다.
#[test]
fn kimi_context_admission_counts_the_complete_tool_projected_payload() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let saw_tools = Arc::new(Mutex::new(false));
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::clone(&requests),
        }),
        kimi_k27_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(ToolAwareBoundaryCounter {
                saw_tools: Arc::clone(&saw_tools),
            }),
        ),
        ModelContextProfile::new(262_144, 32_768, "utf8-bytes/v1").unwrap(),
        Some(kimi_k27_profile()),
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
            input: UserInput::from("count the complete request"),
        })
        .unwrap();

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));
    assert!(*saw_tools.lock().unwrap());
    assert!(requests.lock().unwrap().is_empty());
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
