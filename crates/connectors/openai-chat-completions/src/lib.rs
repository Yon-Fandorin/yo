//! OpenAI-compatible Chat Completions API Connector.

use std::fmt;

use reqwest::{Client, Url};
use serde_json::Value;
use yo_connector_transport::{
    ConnectorStream, DecodeBatch, SseDecoder, configuration_failure, http_client, start_stream,
};
use yo_core::{
    ApiCredential, ApiDialect, ConnectorError, ConnectorId, EffectiveModelBinding, ModelConnector,
    ModelConnectorCancellation, ModelConnectorLimits, ModelConnectorPoll, ModelConnectorRequest,
    ModelConnectorStreamPort,
};

mod request;
mod sse;
#[cfg(test)]
mod tests;

use sse::ChatCompletionsSseDecoder;

impl SseDecoder for ChatCompletionsSseDecoder {
    fn push(&mut self, bytes: &[u8]) -> DecodeBatch {
        self.push_batch(bytes)
    }

    fn finish(&mut self) -> Result<Vec<yo_core::ModelConnectorEvent>, ConnectorError> {
        self.finish()
    }
}

#[derive(Clone)]
pub struct OpenAiChatCompletionsConnector {
    client: Client,
    request_url: Url,
    model: String,
    credential: ApiCredential,
    limits: ModelConnectorLimits,
}

impl OpenAiChatCompletionsConnector {
    pub fn new(
        binding: &EffectiveModelBinding,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
    ) -> Result<Self, ConnectorError> {
        Self::new_with_client_factory(binding, credential, limits, http_client)
    }

    fn new_with_client_factory(
        binding: &EffectiveModelBinding,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
        client_factory: impl FnOnce(&Url, &ModelConnectorLimits) -> Result<Client, ConnectorError>,
    ) -> Result<Self, ConnectorError> {
        limits.validate()?;
        if binding.connector_id().as_str() != ConnectorId::OPENAI_CHAT_COMPLETIONS
            || binding.api_dialect() != ApiDialect::OpenAiChatCompletions
        {
            return Err(configuration_failure(
                "binding does not select the openai-chat-completions connector and dialect",
            ));
        }
        let mut request_url = binding
            .endpoint()
            .append_path_segment("chat")
            .map_err(|_| {
                configuration_failure("cannot append chat/completions to the base endpoint")
            })?;
        request_url
            .path_segments_mut()
            .map_err(|_| {
                configuration_failure("cannot append chat/completions to the base endpoint")
            })?
            .push("completions");
        let client = client_factory(&request_url, &limits)?;
        Ok(Self {
            client,
            request_url,
            model: binding.model_id().as_str().to_owned(),
            credential,
            limits,
        })
    }

    #[must_use]
    pub fn request_url(&self) -> &str {
        self.request_url.as_str()
    }

    pub fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, ConnectorError> {
        request::wire_body(request, &self.model)
    }

    pub fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<ConnectorStream, ConnectorError> {
        let body = request::wire_body(&request, &self.model)?;
        start_stream(
            self.client.clone(),
            self.request_url.clone(),
            self.credential.clone(),
            self.limits.clone(),
            body,
            Box::new(ChatCompletionsSseDecoder::new(self.limits.clone())),
            cancellation,
            "yo-openai-chat-completions",
        )
    }
}

trait ChatCompletionsStream: Send {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), ConnectorError>;
}

impl ChatCompletionsStream for ConnectorStream {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError> {
        self.poll()
    }

    fn cancel(&self) {
        self.cancel();
    }

    fn shutdown(&mut self) -> Result<(), ConnectorError> {
        self.shutdown()
    }
}

struct OpenAiChatCompletionsStream<S = ConnectorStream>(S);

impl<S: ChatCompletionsStream> ModelConnectorStreamPort for OpenAiChatCompletionsStream<S> {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError> {
        self.0.poll()
    }

    fn cancel(&self) {
        self.0.cancel();
    }

    fn shutdown(&mut self) -> Result<(), ConnectorError> {
        self.0.shutdown()
    }
}

impl ModelConnector for OpenAiChatCompletionsConnector {
    fn request_url(&self) -> &str {
        self.request_url()
    }

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, ConnectorError> {
        self.tokenization_payload(request)
    }

    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, ConnectorError> {
        OpenAiChatCompletionsConnector::start(self, request, cancellation).map(|stream| {
            Box::new(OpenAiChatCompletionsStream(stream)) as Box<dyn ModelConnectorStreamPort>
        })
    }
}

impl fmt::Debug for OpenAiChatCompletionsConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiChatCompletionsConnector")
            .field("request_url", &self.request_url)
            .field("model", &self.model)
            .field("credential", &self.credential)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
