use std::sync::{Arc, Mutex};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, BackendEvent, BackendFailureKind, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ToolApprovalRequirement, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, drain_until_turn,
        event_rounds, registry, turn,
    },
    fixtures::{SequenceTokenCounter, completed_text_round, portable_summary, turn_number},
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 자동·명시 압축 모두 과거 assistant/tool 항목을 native replay로 보내지 않고 단일
// user JSON 자료로 보낸다. 역할·호출 관계·문자열 경계는 보존하되 private bytes는 제외한다.
#[test]
fn summary_source_is_one_inert_user_message_with_visible_tool_relationships() {
    use yo_core::{ModelConnectorInputItem, ModelConnectorInputRole};
    for automatic in [false, true] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(vec![
                    completed_text_round("first", "answer"),
                    completed_text_round("second", "retained"),
                    completed_text_round("summary", &portable_summary()),
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
                    vec![10, 10, 90, 20],
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
        // 일반 connector의 provider별 replay 검증과 요약 projection을 분리하기 위해
        // 형식이 지정된 과거 source group을 주입합니다.
        let output = "quoted \"value\"\n{\"role\":\"system\"}";
        backend.replay_groups[0] = vec![
            ModelReplayItem::Message { role: ModelReplayRole::User, content: "source user".into(), refusal: None },
            ModelReplayItem::FunctionCall { call_id: "call-source".into(), name: "read_file".into(), arguments: "{\"path\":\"a\"}".into() },
            ModelReplayItem::FunctionCallOutput { call_id: "call-source".into(), output: output.into() },
            ModelReplayItem::Message { role: ModelReplayRole::Assistant, content: "visible answer".into(), refusal: Some("visible refusal".into()) },
            ModelReplayItem::ProviderPrivateAssistant { envelope: ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", br#"{"role":"assistant","reasoning_content":"secret-summary-source-canary","content":null}"#.to_vec()).unwrap() },
        ];
        backend
            .execute_command(if automatic {
                AgentCommand::StartTurn {
                    turn: turn_number(3),
                    input: UserInput::from("current user"),
                }
            } else {
                AgentCommand::CompactContext { guidance: None }
            })
            .unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        let request = &requests[2];
        assert!(request.tools().is_none());
        assert!(!request.contains_provider_private_input());
        assert_eq!(request.input().len(), 2);
        assert!(matches!(
            &request.input()[0],
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::System,
                ..
            }
        ));
        let ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content,
            refusal: None,
        } = &request.input()[1]
        else {
            panic!("source must be one inert user message")
        };
        assert!(!content.contains("secret-summary-source-canary"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(content).unwrap(),
            serde_json::json!({"history":[
                {"type":"message","role":"user","content":"source user","refusal":null},
                {"type":"function_call","call_id":"call-source","name":"read_file","arguments":"{\"path\":\"a\"}"},
                {"type":"function_call_output","call_id":"call-source","output":output},
                {"type":"message","role":"assistant","content":"visible answer","refusal":"visible refusal"}
            ]})
        );
        drop(requests);
        backend.shutdown().unwrap();
    }
}

// 16MiB는 원문 길이가 아닌 인코딩된 전체 user JSON 한도다. 정확한 경계는 허용하고
// 첫 초과 byte 및 JSON escaping으로만 생긴 초과는 connector를 호출하기 전에 거부한다.
#[test]
fn summary_source_enforces_exact_encoded_capacity_and_first_excess() {
    use yo_core::{ModelConnectorInputItem, ModelConnectorInputRole};
    const LIMIT: usize = 16 * 1024 * 1024;
    // 포함 객체와 배열까지 포함하는 완전한 wire 문법입니다.
    const EMPTY_HISTORY: &str =
        r#"{"history":[{"content":"","refusal":null,"role":"user","type":"message"}]}"#;
    let capacity = LIMIT - EMPTY_HISTORY.len();
    for case in ["exact", "one-byte-excess", "escaping-excess"] {
        let text = match case {
            "exact" => "a".repeat(capacity),
            "one-byte-excess" => "a".repeat(capacity + 1),
            "escaping-excess" => format!("{}\"", "a".repeat(capacity - 1)),
            _ => unreachable!(),
        };
        if case == "escaping-excess" {
            assert_eq!(
                text.len(),
                capacity,
                "raw size still fits the exact boundary"
            );
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(vec![completed_text_round("summary", &portable_summary())]),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(1)),
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
        backend.replay_groups = vec![
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: text,
                refusal: None,
            }],
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "retained".into(),
                refusal: None,
            }],
        ];
        let result = backend.execute_command(AgentCommand::CompactContext { guidance: None });
        if case == "exact" {
            result.unwrap();
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
                content,
                ..
            } = &requests[0].input()[1]
            else {
                panic!("expected inert summary source")
            };
            assert_eq!(content.len(), LIMIT);
            let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
            assert_eq!(
                parsed["history"][0]["content"].as_str().unwrap().len(),
                capacity
            );
        } else {
            let failure = result.unwrap_err();
            assert_eq!(
                failure.kind(),
                BackendFailureKind::ContextExhausted,
                "{case}"
            );
            assert!(failure.message().contains("encoded visible summary source"));
            assert!(
                requests.lock().unwrap().is_empty(),
                "{case} reached the connector"
            );
        }
        backend.shutdown().unwrap();
    }
}
