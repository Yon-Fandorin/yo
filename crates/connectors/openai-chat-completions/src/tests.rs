use yo_core::{
    AccountId, ApiCredential, ApiDialect, ConnectorError, ConnectorFailureKind,
    EffectiveModelBinding, KimiAssistantMessage, ModelConnectorCancellation, ModelConnectorEvent,
    ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorLimits, ModelConnectorPoll,
    ModelConnectorRequest, ModelConnectorStreamPort, ModelId, NormalizedEndpoint, ProviderId,
    RequestToolExposure,
};

use super::*;

fn chat_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("token-plan").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://dashscope-intl.aliyuncs.com/compatible-mode/v1")
            .unwrap(),
    )
}

fn request() -> ModelConnectorRequest {
    ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        128,
        None,
    )
    .unwrap()
}

// Chat Connector는 정확한 두 path segment를 붙이고 credential 원문은 Debug에서 숨깁니다.
#[test]
fn appends_exact_chat_completions_path_and_redacts_credentials() {
    let connector = OpenAiChatCompletionsConnector::new(
        &chat_binding(),
        ApiCredential::new("secret-token").unwrap(),
        ModelConnectorLimits::default(),
    )
    .unwrap();

    assert_eq!(
        connector.request_url(),
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
    assert!(!format!("{connector:?}").contains("secret-token"));
}

// 다른 dialect binding은 endpoint나 HTTP client를 사용하기 전에 typed configuration failure가
// 됩니다.
#[test]
fn rejects_a_binding_for_a_different_connector_identity() {
    let binding = EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("token-plan").unwrap(),
        ModelId::new("model").unwrap(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
    );

    let error = OpenAiChatCompletionsConnector::new(
        &binding,
        ApiCredential::new("secret").unwrap(),
        ModelConnectorLimits::default(),
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}

// 이미 취소된 token은 network request를 시작하기 전에 Cancelled failure로 끝납니다.
#[test]
fn rejects_a_cancelled_request_before_network_work() {
    let connector = OpenAiChatCompletionsConnector::new(
        &chat_binding(),
        ApiCredential::new("secret-token").unwrap(),
        ModelConnectorLimits::default(),
    )
    .unwrap();
    let cancellation = ModelConnectorCancellation::new();
    cancellation.cancel();

    let error = connector.start(request(), cancellation).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Cancelled);
}

// 0인 finite limit는 사실상 무제한 profile로 해석하지 않고 client 생성 전에 거절합니다.
#[test]
fn rejects_a_connector_profile_with_a_zero_bound() {
    let limits = ModelConnectorLimits {
        max_sse_events: 0,
        ..ModelConnectorLimits::default()
    };

    let error = OpenAiChatCompletionsConnector::new(
        &chat_binding(),
        ApiCredential::new("secret-token").unwrap(),
        limits,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}

// Kimi private assistant input은 generic Chat body로 축약하지 않고 dispatch 전에 거절합니다.
#[test]
fn rejects_provider_private_assistant_replay_before_dispatch() {
    let connector = OpenAiChatCompletionsConnector::new(
        &chat_binding(),
        ApiCredential::new("secret-token").unwrap(),
        ModelConnectorLimits::default(),
    )
    .unwrap();
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::ProviderPrivateAssistant {
            schema: "kimi.assistant-message/v1alpha1".to_owned(),
            message: KimiAssistantMessage::new("private", Some("visible".to_owned()), Vec::new()),
        }],
        RequestToolExposure::disabled(),
        128,
        None,
    )
    .unwrap();

    let error = connector.tokenization_payload(&request).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}

struct ScriptedStream {
    polls: std::collections::VecDeque<Result<ModelConnectorPoll, ConnectorError>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    shutdown: bool,
}

impl ChatCompletionsStream for ScriptedStream {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError> {
        self.polls
            .pop_front()
            .expect("the test polls only the scripted observations")
    }

    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn shutdown(&mut self) -> Result<(), ConnectorError> {
        self.shutdown = true;
        Ok(())
    }
}

// neutral stream port wrapper는 queued event를 순서대로 내보낸 뒤 Closed를 전달하고 cleanup을
// 위임합니다.
#[test]
fn neutral_stream_port_preserves_polling_and_cleanup_order() {
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let terminal = ModelConnectorEvent::Terminal {
        response_id: "chat-1".to_owned(),
        status: yo_core::ModelConnectorTerminal::Completed,
        usage: yo_core::ModelConnectorUsage {
            input_tokens: Some(4),
            output_tokens: Some(3),
            total_tokens: Some(7),
            reasoning_tokens: None,
        },
    };
    let mut stream = OpenAiChatCompletionsStream(ScriptedStream {
        polls: [
            Ok(ModelConnectorPoll::Event(
                ModelConnectorEvent::ResponseCreated {
                    response_id: "chat-1".to_owned(),
                },
            )),
            Ok(ModelConnectorPoll::Event(terminal.clone())),
            Ok(ModelConnectorPoll::Closed),
        ]
        .into(),
        cancelled: cancelled.clone(),
        shutdown: false,
    });

    assert!(matches!(
        ModelConnectorStreamPort::poll(&mut stream),
        Ok(ModelConnectorPoll::Event(
            ModelConnectorEvent::ResponseCreated { .. }
        ))
    ));
    assert_eq!(
        ModelConnectorStreamPort::poll(&mut stream),
        Ok(ModelConnectorPoll::Event(terminal))
    );
    assert_eq!(
        ModelConnectorStreamPort::poll(&mut stream),
        Ok(ModelConnectorPoll::Closed)
    );
    ModelConnectorStreamPort::cancel(&stream);
    ModelConnectorStreamPort::shutdown(&mut stream).unwrap();
    assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
    assert!(stream.0.shutdown);
}
