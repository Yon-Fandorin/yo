//! OpenAI Responses API Connector.

use std::fmt;

use reqwest::{Client, Url};
use serde_json::{Value, json};
use yo_connector_transport::{
    ConnectorStream, DecodeBatch, SseDecoder, configuration_failure, http_client, start_stream,
};
use yo_core::{
    ApiCredential, ApiDialect, ConnectorError, ConnectorId, EffectiveModelBinding, ModelConnector,
    ModelConnectorCancellation, ModelConnectorInputItem, ModelConnectorLimits, ModelConnectorPoll,
    ModelConnectorRequest, ModelConnectorStreamPort,
};

mod sse;
#[cfg(test)]
mod tests;

use sse::ResponsesSseDecoder;

impl SseDecoder for ResponsesSseDecoder {
    fn push(&mut self, bytes: &[u8]) -> DecodeBatch {
        self.push_batch(bytes)
    }
    fn finish(&mut self) -> Result<Vec<yo_core::ModelConnectorEvent>, ConnectorError> {
        self.finish()
    }
}

#[derive(Clone)]
pub struct OpenAiResponsesConnector {
    client: Client,
    request_url: Url,
    model: String,
    credential: ApiCredential,
    limits: ModelConnectorLimits,
}

impl OpenAiResponsesConnector {
    pub fn new(
        binding: &EffectiveModelBinding,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
    ) -> Result<Self, ConnectorError> {
        Self::new_with_client_factory(binding, credential, limits, http_client)
    }

    #[cfg(test)]
    fn new_with_test_root(
        binding: &EffectiveModelBinding,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
        root_pem: &[u8],
    ) -> Result<Self, ConnectorError> {
        Self::new_with_client_factory(binding, credential, limits, |request_url, limits| {
            yo_connector_transport::http_client_with_test_root(request_url, limits, root_pem)
        })
    }

    fn new_with_client_factory(
        binding: &EffectiveModelBinding,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
        client_factory: impl FnOnce(&Url, &ModelConnectorLimits) -> Result<Client, ConnectorError>,
    ) -> Result<Self, ConnectorError> {
        limits.validate()?;
        if binding.connector_id().as_str() != ConnectorId::OPENAI_RESPONSES
            || binding.api_dialect() != ApiDialect::OpenAiResponses
        {
            return Err(configuration_failure(
                "binding does not select the openai-responses connector and protocol",
            ));
        }
        let request_url = binding
            .endpoint()
            .append_path_segment("responses")
            .map_err(|_| configuration_failure("cannot append responses to the base endpoint"))?;
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

    fn wire_body(&self, request: &ModelConnectorRequest) -> Result<Value, ConnectorError> {
        if request.contains_provider_private_input() {
            return Err(configuration_failure(
                "provider-private assistant replay requires its provider-specific connector",
            ));
        }
        let input = request
            .input()
            .iter()
            .map(|item| match item {
                ModelConnectorInputItem::Message {
                    role,
                    content,
                    refusal,
                } => {
                    let mut visible = content.clone();
                    if let Some(refusal) = refusal {
                        visible.push_str(refusal);
                    }
                    json!({"role": role.as_str(), "content": visible})
                },
                ModelConnectorInputItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                } => json!({
                    "type": "function_call", "call_id": call_id, "name": name,
                    "arguments": arguments,
                }),
                ModelConnectorInputItem::FunctionCallOutput { call_id, output } => json!({
                    "type": "function_call_output", "call_id": call_id, "output": output,
                }),
                ModelConnectorInputItem::ProviderPrivateAssistant { .. } => unreachable!(
                    "provider-private input was rejected before Responses serialization"
                ),
            })
            .collect::<Vec<_>>();
        let mut body = json!({"model": self.model, "input": input, "stream": true});
        if let Some(maximum) = request.max_output_tokens() {
            body["max_output_tokens"] = Value::from(maximum);
        }
        if let Some(tools) = request.tools() {
            body["tools"] = Value::Array(
                tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function", "name": tool.name(),
                            "description": tool.description(), "parameters": tool.parameters(),
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = Value::String("auto".to_owned());
        }
        if let Some(effort) = request.reasoning_effort() {
            body["reasoning"] = json!({"effort": effort.as_str()});
        }
        Ok(body)
    }

    pub fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, ConnectorError> {
        self.wire_body(request)
    }

    pub fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<yo_connector_transport::ConnectorStream, ConnectorError> {
        let body = self.wire_body(&request)?;
        start_stream(
            self.client.clone(),
            self.request_url.clone(),
            self.credential.clone(),
            self.limits.clone(),
            body,
            Box::new(ResponsesSseDecoder::new(self.limits.clone())),
            cancellation,
            "yo-openai-responses",
        )
    }
}

struct OpenAiResponsesStream(ConnectorStream);

impl ModelConnectorStreamPort for OpenAiResponsesStream {
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

impl ModelConnector for OpenAiResponsesConnector {
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
        OpenAiResponsesConnector::start(self, request, cancellation).map(|stream| {
            Box::new(OpenAiResponsesStream(stream)) as Box<dyn ModelConnectorStreamPort>
        })
    }
}

impl fmt::Debug for OpenAiResponsesConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiResponsesConnector")
            .field("request_url", &self.request_url)
            .field("model", &self.model)
            .field("credential", &self.credential)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
