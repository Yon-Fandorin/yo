use std::sync::{Arc, Mutex};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendEvent, BackendPoll, ModelConnectorEvent,
        ModelConnectorInputItem, ModelReplayItem, NativeModelBackend, NativeModelBackendConfig,
        NativeModelBackendServices, TOOL_TRUNCATION_MARKER, ToolApprovalRequirement, ToolExecution,
        ToolExecutionHost, ToolExecutionRequest, ToolSemanticAdmission, ToolValidationFailure,
        bounded_output,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, completed,
        context_profile, event_rounds, registry, turn,
    },
};
use crate::{ToolExecutionError, ToolId, UserInput};

struct FailingStartHost;

impl ToolExecutionHost for FailingStartHost {
    fn identity(&self) -> &str {
        "failing-start-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        Err(ToolExecutionError::new("execution-host-secret"))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

struct RedactingAdmission;

impl ToolSemanticAdmission for RedactingAdmission {
    fn admit_arguments(
        &self,
        _definition: &crate::ToolDefinition,
        _validated_argument_bytes: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Ok(r#"{"path":"[redacted]"}"#.to_owned())
    }

    fn admit_output(
        &self,
        _definition: &crate::ToolDefinition,
        _bounded_output: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Ok("[redacted-output]".to_owned())
    }
}

struct RejectingAdmission;

impl ToolSemanticAdmission for RejectingAdmission {
    fn admit_arguments(
        &self,
        _definition: &crate::ToolDefinition,
        _validated_argument_bytes: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Err(crate::ToolSemanticAdmissionError::new(
            "host diagnostic contains secret.txt",
        ))
    }

    fn admit_output(
        &self,
        _definition: &crate::ToolDefinition,
        _bounded_output: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Err(crate::ToolSemanticAdmissionError::new(
            "host diagnostic contains output-secret",
        ))
    }
}

struct FailingTokenCounter;

impl crate::ModelTokenCounter for FailingTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        _request: &serde_json::Value,
    ) -> Result<u64, crate::ModelTokenCounterError> {
        Err(crate::ModelTokenCounterError::new(
            "counter diagnostic contains request-secret",
        ))
    }
}

// host가 이미 잘랐다고 보고하거나 UTF-8 경계에서 다시 잘라도 truncation marker를 포함한
// 최종 model-visible output 자체가 설정한 byte limit을 넘지 않는다.
#[test]
fn bounded_tool_output_includes_its_marker_inside_the_limit() {
    let bounded = bounded_output("가나다라마바사", 32, true);
    assert!(bounded.len() <= 32);
    assert!(bounded.ends_with("[yo: tool output truncated]"));

    let tiny = bounded_output("가나다", 5, false);
    assert!(tiny.len() <= 5);
    assert!(tiny.is_char_boundary(tiny.len()));

    let invalid = NativeModelBackendConfig {
        maximum_tool_output_bytes: TOOL_TRUNCATION_MARKER.len() - 1,
        ..NativeModelBackendConfig::default()
    };
    assert!(
        NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(Vec::new()),
                requests: Arc::new(Mutex::new(Vec::new())),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(1)),
            ),
            context_profile(),
            invalid,
        )
        .is_err()
    );
}

// local tool을 노출하면 원시 argument와 output이 semantic 경계를 우회하지 못하도록 startup을
// 거부하는지 검증합니다.
#[test]
fn native_backend_refuses_to_expose_tools_without_semantic_admission() {
    let result = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            None,
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    );

    let Err(error) = result else {
        panic!("a non-empty registry without semantic admission must fail")
    };
    assert!(error.message().contains("semantic-admission"));
}

// host가 admission한 값만 Activity·replay·다음 request에 남고 원시 tool 값은 노출되지 않는지
// 검증합니다.
#[test]
fn semantic_admission_replaces_tool_values_before_activity_replay_and_next_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"secret.txt"}"#.to_owned(),
            },
            completed("r1"),
        ],
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r2".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("r2"),
        ],
    ];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(RedactingAdmission)),
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
            input: UserInput::from("read secret"),
        })
        .unwrap();

    let mut visible_updates = Vec::new();
    let evidence = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: crate::ActivityUpdate::TextSnapshot(text),
                ..
            }) => visible_updates.push(text),
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { evidence, .. }) => {
                break evidence;
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before resumable completion"),
        }
    };
    let visible = visible_updates.join("\n");
    assert!(!visible.contains("secret.txt"));
    assert!(!visible.contains("contents"));
    assert!(visible.contains("[redacted]"));
    assert!(visible.contains("[redacted-output]"));

    let replay = evidence.model_replay().unwrap();
    assert!(replay.items().iter().any(|item| matches!(
        item,
        ModelReplayItem::FunctionCall { arguments, .. }
            if arguments == r#"{"path":"[redacted]"}"#
    )));
    assert!(replay.items().iter().any(|item| matches!(
        item,
        ModelReplayItem::FunctionCallOutput { output, .. }
            if output == "[redacted-output]"
    )));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input().iter().any(|item| matches!(
        item,
        ModelConnectorInputItem::FunctionCall { arguments, .. }
            if arguments == r#"{"path":"[redacted]"}"#
    )));
    assert!(requests[1].input().iter().any(|item| matches!(
        item,
        ModelConnectorInputItem::FunctionCallOutput { output, .. }
            if output == "[redacted-output]"
    )));
}

// Schema mismatch 상세 설명에 model이 만든 property name이 들어가더라도, 안정적인 backend 소유
// 진단만 semantic event에 남는지 검증합니다.
#[test]
fn schema_validation_diagnostics_do_not_persist_argument_property_names() {
    let rounds = vec![vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"secret-property":"value"}"#.to_owned(),
        },
    ]];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
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
            input: UserInput::from("request"),
        })
        .unwrap();

    let mut visible = String::new();
    loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => {
                visible.push_str(&format!("{event:?}"));
                break;
            },
            BackendPoll::Event(event) => visible.push_str(&format!("{event:?}")),
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the failed Turn"),
        }
    }

    assert!(!visible.contains("secret-property"));
    assert!(visible.contains("tool arguments do not match the admitted schema"));
    assert!(visible.contains(ToolValidationFailure::SchemaMismatch.code()));
}

// 주입한 admission·tokenizer의 실패 설명이 금지된 원문을 담아도 semantic event와 caller error에
// 노출되지 않는지 검증합니다.
#[test]
fn injected_policy_diagnostics_do_not_cross_the_semantic_boundary() {
    let rounds = vec![vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"secret.txt"}"#.to_owned(),
        },
        completed("r1"),
    ]];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(RejectingAdmission)),
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
            input: UserInput::from("request"),
        })
        .unwrap();

    let mut visible = String::new();
    loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => {
                visible.push_str(&format!("{event:?}"));
                break;
            },
            BackendPoll::Event(event) => visible.push_str(&format!("{event:?}")),
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the failed Turn"),
        }
    }
    assert!(!visible.contains("secret.txt"));
    assert!(visible.contains("semantic admission was rejected"));

    let mut counter_backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FailingTokenCounter),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    counter_backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    let error = counter_backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("request-secret"),
        })
        .unwrap_err();
    assert!(!error.to_string().contains("request-secret"));
    assert!(error.to_string().contains("token counting failed"));

    let rounds = vec![
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "tool".to_owned(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            completed("tool"),
        ],
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "answer".to_owned(),
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
            completed("answer"),
        ],
    ];
    let mut host_backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(FailingStartHost),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    host_backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    host_backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("request"),
        })
        .unwrap();
    let mut visible = String::new();
    loop {
        match host_backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::ResumableTurnFinished { .. }) => {
                visible.push_str(&format!("{event:?}"));
                break;
            },
            BackendPoll::Event(event) => visible.push_str(&format!("{event:?}")),
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the completed Turn"),
        }
    }
    assert!(!visible.contains("execution-host-secret"));
    assert!(visible.contains("tool execution failed"));
}
