use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AccountId, ActivityKind, ActivityOutcome, ActivityUpdate, AgentCommand, ApiDialect,
    BackendCommandEvidence, BackendEvent, BackendPoll, EffectiveModelBinding, ModelConnectorEvent,
    ModelConnectorTerminal, ModelContextProfile, ModelId, ModelProfileLayer,
    ModelProfileParameters, ModelReplayItem, ModelReplayRole, NormalizedEndpoint, ProviderId,
    ProviderPrivateReplayEnvelope, ReasoningChannel, ReplayProfile, ToolApprovalRequirement,
    TurnOutcome, UserInput, VersionedProfileId,
};

use super::support::{
    ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, completed,
    context_profile, drain_until_turn, event_rounds,
    kimi::{kimi_admission, kimi_binding, kimi_profile, private_envelope, visible_message},
    mock_tokenization_payload, registry, turn,
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

fn chat_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://example.invalid/v1").unwrap(),
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

fn kimi_k27_profile() -> yo_core::EffectiveModelProfile {
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
    )
    .with_replay_profile(Some(
        VersionedProfileId::new("kimi-private-local-plaintext/v1").unwrap(),
    ));
    yo_core::EffectiveModelProfile::resolve(None, &layer).unwrap()
}

fn started_private_backend() -> NativeModelBackend {
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        kimi_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(kimi_admission),
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
            input: UserInput::from("validate private replay ordering"),
        })
        .unwrap();
    backend
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
            envelope: private_envelope(private, Some(visible)),
            visible_projection: vec![visible_message(visible)],
        },
        completed(response),
    ]
}

fn completed_round_events(round: Vec<ModelConnectorEvent>) -> Vec<BackendEvent> {
    let mut backend = backend(
        vec![round],
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
            input: UserInput::from("fixture"),
        })
        .unwrap();

    let mut events = Vec::new();
    for _ in 0..100 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event) => {
                let finished = matches!(
                    event,
                    BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
                );
                events.push(event);
                if finished {
                    return events;
                }
            },
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before finishing the Turn"),
        }
    }
    panic!("backend did not finish within the deterministic poll budget")
}

// managed Connector의 reasoning은 ModelWork로 남기되 visible assistant content는 content
// part 수와 무관하게 하나의 completed AgentMessage로 투영해 print mode가 최종 답을 찾습니다.
#[test]
fn native_backend_projects_visible_assistant_as_one_agent_message() {
    let events = completed_round_events(vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "response-1".to_owned(),
        },
        ModelConnectorEvent::ReasoningDelta {
            output_index: 0,
            item_id: "reasoning".to_owned(),
            channel: ReasoningChannel::Summary,
            part_index: 0,
            delta: "thinking".to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 1,
            item_id: "message".to_owned(),
            content_index: 0,
            delta: "answer".to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 1,
            item_id: "message".to_owned(),
            content_index: 1,
            delta: "!".to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 1,
            item_id: "message".to_owned(),
        },
        completed("response-1"),
    ]);
    let agent_activities = events
        .iter()
        .filter_map(|event| match event {
            BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::AgentMessage,
            } => Some(*activity),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(agent_activities.len(), 1);
    let agent = agent_activities[0];
    let text = events
        .iter()
        .filter_map(|event| match event {
            BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextDelta(delta),
            } if *activity == agent => Some(delta.as_str()),
            _ => None,
        })
        .collect::<String>();

    assert!(events.iter().any(|event| matches!(
        event,
        BackendEvent::ActivityStarted {
            kind: ActivityKind::ModelWork,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        } if *activity == agent
    )));
    assert_eq!(text, "answer!");
}

// content delta가 없는 의도적인 빈 assistant item도 final-message identity를 잃지 않고
// 빈 completed AgentMessage로 투영합니다.
#[test]
fn native_backend_projects_empty_assistant_item_as_agent_message() {
    let events = completed_round_events(vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "empty".to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: "message".to_owned(),
        },
        completed("empty"),
    ]);
    let agent = events.iter().find_map(|event| match event {
        BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        } => Some(*activity),
        _ => None,
    });
    assert!(
        agent.is_some_and(|agent| events.iter().any(|event| matches!(
            event,
            BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            } if *activity == agent
        )))
    );
}

struct ToolAwareBoundaryCounter {
    saw_tools: Arc<Mutex<bool>>,
}

impl yo_core::ModelTokenCounter for ToolAwareBoundaryCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        payload: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
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
            Box::new(kimi_admission),
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
        yo_core::ContinuationStrategy::ExactReplay {
            replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
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
                ModelReplayItem::ProviderPrivateAssistant { envelope }
                    if std::str::from_utf8(envelope.payload()).unwrap().contains("hidden-1")
            ))
    );

    let second_turn = yo_core::TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second request"),
        })
        .unwrap();
    let _ = drain_until_turn(&mut backend);
    let requests = requests.lock().unwrap();
    let expected_cache_hint = turn().session_id().to_string();
    assert_eq!(
        requests[0].cache_affinity_hint(),
        Some(expected_cache_hint.as_str())
    );
    assert_eq!(
        requests[1].cache_affinity_hint(),
        Some(expected_cache_hint.as_str())
    );
    assert!(requests[1].input().iter().any(|item| matches!(
        item,
        yo_core::ModelConnectorInputItem::ProviderPrivateAssistant { envelope }
            if std::str::from_utf8(envelope.payload()).unwrap().contains("hidden-1")
    )));
}

// opaque payload를 해석하지 않는 backend도 completed projection, exact replay schema,
// terminal sealing 순서를 강제해 partial/later semantic output을 Anchor 후보로 만들지 않습니다.
#[test]
fn native_backend_seals_provider_private_only_after_complete_exact_projection() {
    let mut backend = started_private_backend();
    let mut state = backend.turn.take().unwrap();
    backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "visible".to_owned(),
            },
        )
        .unwrap();

    let private = || ModelConnectorEvent::ProviderPrivateAssistant {
        output_index: 1,
        envelope: private_envelope("hidden", Some("visible")),
        visible_projection: vec![visible_message("visible")],
    };
    assert!(backend.apply_response_event(&mut state, private()).is_err());

    backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
        )
        .unwrap();
    let wrong_schema = ModelConnectorEvent::ProviderPrivateAssistant {
        output_index: 1,
        envelope: ProviderPrivateReplayEnvelope::new(
            "other.private/v1",
            br#"{"opaque":true}"#.to_vec(),
        )
        .unwrap(),
        visible_projection: vec![visible_message("visible")],
    };
    assert!(
        backend
            .apply_response_event(&mut state, wrong_schema)
            .is_err()
    );

    let wrong_projection = ModelConnectorEvent::ProviderPrivateAssistant {
        output_index: 1,
        envelope: private_envelope("hidden", Some("visible")),
        visible_projection: vec![visible_message("different")],
    };
    assert!(
        backend
            .apply_response_event(&mut state, wrong_projection)
            .is_err()
    );
    backend.apply_response_event(&mut state, private()).unwrap();
    assert!(
        backend
            .apply_response_event(
                &mut state,
                ModelConnectorEvent::TextDelta {
                    output_index: 0,
                    item_id: "message".to_owned(),
                    content_index: 0,
                    delta: "late".to_owned(),
                },
            )
            .is_err()
    );
}

// visible group이 없거나 private index가 완료된 call보다 앞서면 저장 전에 거부합니다.
#[test]
fn native_backend_rejects_orphan_and_misordered_provider_private_items() {
    let mut backend = started_private_backend();
    let mut orphan = backend.turn.take().unwrap();
    assert!(
        backend
            .apply_response_event(
                &mut orphan,
                ModelConnectorEvent::ProviderPrivateAssistant {
                    output_index: 0,
                    envelope: private_envelope("hidden", None),
                    visible_projection: Vec::new(),
                },
            )
            .is_err()
    );
    assert!(orphan.round_replay.is_empty());

    orphan.round_message_items.insert(0);
    orphan.round_replay.insert(
        2,
        ModelReplayItem::FunctionCall {
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
    );
    assert!(
        backend
            .apply_response_event(
                &mut orphan,
                ModelConnectorEvent::ProviderPrivateAssistant {
                    output_index: 1,
                    envelope: private_envelope("hidden", None),
                    visible_projection: vec![
                        visible_message(""),
                        ModelReplayItem::FunctionCall {
                            call_id: "call-1".to_owned(),
                            name: "read_file".to_owned(),
                            arguments: r#"{"path":"README.md"}"#.to_owned(),
                        },
                    ],
                },
            )
            .is_err()
    );
    assert!(!orphan.round_replay.contains_key(&1));
}

// private profile의 단일 completed round가 envelope를 생략하면 resumable 완료를 만들지 않습니다.
#[test]
fn private_profile_fails_a_completed_round_missing_its_private_item() {
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "kimi-missing-private".to_owned(),
                },
                ModelConnectorEvent::MessageDone {
                    output_index: 0,
                    item_id: "message".to_owned(),
                },
                completed("kimi-missing-private"),
            ]]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        kimi_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(kimi_admission),
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
            input: UserInput::from("require private replay"),
        })
        .unwrap();

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
}

// 앞 tool round가 유효해도 뒤 completed assistant의 private 누락을 가리지 못하는지 검증합니다.
#[test]
fn private_profile_fails_a_later_round_missing_private_after_a_valid_tool_round() {
    let call = ModelReplayItem::FunctionCall {
        call_id: "call-1".to_owned(),
        name: "read_file".to_owned(),
        arguments: r#"{"path":"README.md"}"#.to_owned(),
    };
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                vec![
                    ModelConnectorEvent::ResponseCreated {
                        response_id: "kimi-tool".to_owned(),
                    },
                    ModelConnectorEvent::MessageDone {
                        output_index: 0,
                        item_id: "message".to_owned(),
                    },
                    ModelConnectorEvent::FunctionCallStarted {
                        output_index: 1,
                        item_id: "call-item".to_owned(),
                        call_id: "call-1".to_owned(),
                        name: "read_file".to_owned(),
                    },
                    ModelConnectorEvent::FunctionCallDone {
                        output_index: 1,
                        item_id: "call-item".to_owned(),
                        call_id: "call-1".to_owned(),
                        name: "read_file".to_owned(),
                        arguments: r#"{"path":"README.md"}"#.to_owned(),
                    },
                    ModelConnectorEvent::ProviderPrivateAssistant {
                        output_index: 2,
                        envelope: private_envelope("hidden-tool", None),
                        visible_projection: vec![visible_message(""), call],
                    },
                    completed("kimi-tool"),
                ],
                vec![
                    ModelConnectorEvent::ResponseCreated {
                        response_id: "kimi-final".to_owned(),
                    },
                    ModelConnectorEvent::TextDelta {
                        output_index: 0,
                        item_id: "message".to_owned(),
                        content_index: 0,
                        delta: "final".to_owned(),
                    },
                    ModelConnectorEvent::MessageDone {
                        output_index: 0,
                        item_id: "message".to_owned(),
                    },
                    completed("kimi-final"),
                ],
            ]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        kimi_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(kimi_admission),
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
            input: UserInput::from("tool then final"),
        })
        .unwrap();

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
}

// 시작만 된 function call보다 private envelope가 먼저 오면 partial projection으로 받지 않습니다.
#[test]
fn native_backend_rejects_provider_private_before_function_call_done() {
    let mut backend = started_private_backend();
    let mut state = backend.turn.take().unwrap();
    backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
        )
        .unwrap();
    backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 1,
                item_id: "call-item".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
        )
        .unwrap();
    let event = ModelConnectorEvent::ProviderPrivateAssistant {
        output_index: 2,
        envelope: private_envelope("hidden", None),
        visible_projection: vec![visible_message("")],
    };
    assert!(backend.apply_response_event(&mut state, event).is_err());
}

// K2.7의 tool 포함 payload가 229377 tokens이면 hard max 32768은 넘치므로 최종 payload를
// cap 32767로 다시 세어 정확히 262144에 맞춘 요청 한 건만 전송합니다.
#[test]
fn kimi_context_admission_selects_a_smaller_cap_from_the_complete_tool_payload() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let saw_tools = Arc::new(Mutex::new(false));
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::clone(&requests),
        }),
        kimi_k27_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(kimi_admission),
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

    assert!(*saw_tools.lock().unwrap());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        mock_tokenization_payload(&requests[0], "kimi-k2.7-code")["max_output_tokens"],
        32_767
    );
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
            Box::new(yo_core::admit_standard_complete_binding),
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
                    request_failure: yo_core::ModelRequestFailureKind::ResponseLimit,
                },
                usage: yo_core::ModelConnectorUsage::default(),
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

// 미완성 답변·도구 호출 뒤의 제한/실패 terminal은 원인과 usage를 남기고 도구·replay를 실행하지
// 않는다. 성공 terminal의 미완성 호출은 계속 protocol 오류이며 모든 시작 activity는 한 번만 닫힌다.
#[test]
fn failed_terminals_preserve_partial_output_without_executing_unfinished_calls() {
    use std::collections::HashSet;

    use yo_core::{ActivityNotice, ModelConnectorUsage, ModelRequestFailureKind, NoticeLevel};
    for (status, expected, limited) in [
        (
            ModelConnectorTerminal::Incomplete {
                reason: Some("length".into()),
                request_failure: ModelRequestFailureKind::ResponseLimit,
            },
            "model response was incomplete: length",
            true,
        ),
        (
            ModelConnectorTerminal::Incomplete {
                reason: Some("max_output_tokens".into()),
                request_failure: ModelRequestFailureKind::ResponseLimit,
            },
            "model response was incomplete: max_output_tokens",
            true,
        ),
        (
            ModelConnectorTerminal::Failed {
                code: Some("provider_busy".into()),
                request_failure: ModelRequestFailureKind::RateLimited,
            },
            "model response failed: provider_busy",
            false,
        ),
        (
            ModelConnectorTerminal::Completed,
            "model terminal arrived with an incomplete function call",
            false,
        ),
    ] {
        let starts = Arc::new(Mutex::new(0));
        let completed = matches!(status, ModelConnectorTerminal::Completed);
        let mut backend = backend(
            vec![vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "partial".into(),
                },
                ModelConnectorEvent::TextDelta {
                    output_index: 0,
                    item_id: "message".into(),
                    content_index: 0,
                    delta: "partial answer retained".into(),
                },
                ModelConnectorEvent::FunctionCallStarted {
                    output_index: 1,
                    item_id: "call-item".into(),
                    call_id: "call-1".into(),
                    name: "read_file".into(),
                },
                ModelConnectorEvent::Terminal {
                    response_id: "partial".into(),
                    status,
                    usage: ModelConnectorUsage::default(),
                },
            ]],
            ToolApprovalRequirement::Automatic,
            starts.clone(),
        );
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("partial"),
            })
            .unwrap();
        let mut open = HashSet::new();
        let mut partial = false;
        let mut notices = 0;
        let mut receipts = 0;
        let mut terminal = false;
        for _ in 0..100 {
            match backend.poll_event().unwrap() {
                BackendPoll::Event(BackendEvent::ActivityStarted { activity, .. }) => {
                    assert!(open.insert(activity));
                },
                BackendPoll::Event(BackendEvent::ActivityUpdated { update, .. }) => {
                    let text = match update {
                        ActivityUpdate::TextDelta(text) | ActivityUpdate::TextSnapshot(text) => {
                            text
                        },
                    };
                    partial |= text.contains("partial answer retained");
                    if let Some(notice) = ActivityNotice::from_snapshot(&text) {
                        assert_eq!(notice.title, "Response limit reached");
                        assert_eq!(notice.level, NoticeLevel::Warning);
                        notices += 1;
                    }
                    receipts += usize::from(text.contains("yo.model-usage-receipt/v1"));
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. }) => {
                    assert!(open.remove(&activity));
                },
                BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                    panic!("partial round must not publish replay")
                },
                BackendPoll::Event(BackendEvent::TurnFinished { outcome, .. }) => {
                    let TurnOutcome::Failed(failure) = outcome else {
                        panic!("partial round must fail")
                    };
                    assert!(
                        failure.message().contains(expected),
                        "{}",
                        failure.message()
                    );
                    terminal = true;
                    break;
                },
                _ => {},
            }
        }
        assert!(terminal && partial);
        assert!(open.is_empty());
        assert_eq!(notices, usize::from(limited));
        assert_eq!(receipts, usize::from(!completed));
        assert_eq!(*starts.lock().unwrap(), 0);
    }
}

// managed provider 요청과 토큰 계산은 같은 전체 스킬 지침을 포함하며 완료 replay도 이를 보존한다.
#[test]
fn resolved_skill_snapshot_is_counted_sent_and_retained_in_native_replay() {
    use yo_core::{
        InputReference, ModelConnectorInputItem, ModelTokenCounter, ModelTokenCounterError,
        ResolvedSkill, SkillReference, SkillReferenceScope,
    };

    use super::support::binding;
    let reference = SkillReference::new(
        "review",
        "host",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let input = UserInput::with_references(
        "use $review",
        vec![InputReference::skill(4..11, reference.clone())],
    )
    .unwrap()
    .with_resolved_skill(
        ResolvedSkill::new(reference, "# Review\nVerify the exact implementation.").unwrap(),
    )
    .unwrap();
    let expected = input.model_input().to_owned();
    struct Counter(Arc<Mutex<Vec<serde_json::Value>>>);
    impl ModelTokenCounter for Counter {
        fn count_input_tokens(
            &self,
            _: &str,
            request: &serde_json::Value,
        ) -> Result<u64, ModelTokenCounterError> {
            self.0.lock().unwrap().push(request.clone());
            Ok(1)
        }
    }
    let counted = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "skill-response".to_owned(),
                },
                ModelConnectorEvent::TextDelta {
                    output_index: 0,
                    item_id: "answer".to_owned(),
                    content_index: 0,
                    delta: "done".to_owned(),
                },
                ModelConnectorEvent::MessageDone {
                    output_index: 0,
                    item_id: "answer".to_owned(),
                },
                completed("skill-response"),
            ]]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(Counter(Arc::clone(&counted))),
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
            input,
        })
        .unwrap();
    let completion = drain_until_turn(&mut backend);
    let BackendEvent::ResumableTurnFinished { evidence, .. } = completion else {
        panic!("resumable completion: {completion:?}")
    };
    assert!(
        requests.lock().unwrap()[0]
            .input()
            .iter()
            .any(|item| matches!(item,
        ModelConnectorInputItem::Message { content, .. } if content == &expected))
    );
    assert!(counted.lock().unwrap().iter().any(|request| {
        request["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["content"] == expected)
    }));
    assert!(evidence.model_replay().unwrap().items().iter().any(|item| matches!(item,
        ModelReplayItem::Message { role: ModelReplayRole::User, content, .. } if content == &expected)));
}
