//! Kimi Chat Completions Connector and provider-private replay codec.

use std::fmt;

use reqwest::{Client, Url};
use serde_json::Value;
use yo_connector_transport::{
    ConnectorStream, DecodeBatch, SseDecoder, configuration_failure, http_client, start_stream,
};
use yo_core::{
    ApiCredential, CompleteModelBinding, ConnectorError, ModelConnector,
    ModelConnectorCancellation, ModelConnectorLimits, ModelConnectorPoll, ModelConnectorRequest,
    ModelConnectorStreamPort,
};

mod private_replay;
mod request;
mod sse;
#[cfg(test)]
mod tests;

use request::KimiWireProfile;
use sse::ChatCompletionsSseDecoder;

const KIMI_MAX_REDIRECTS: usize = 3;
const KIMI_MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const KIMI_MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;
const KIMI_MAX_SSE_EVENTS: usize = 100_000;
const KIMI_MAX_OUTPUT_ITEMS: usize = 1_024;
const KIMI_MAX_RESPONSE_TEXT_BYTES: usize = 16 * 1024 * 1024;
const KIMI_MAX_REASONING_BYTES: usize = 16 * 1024 * 1024;
const KIMI_MAX_FUNCTION_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;
const KIMI_MAX_PROVIDER_PRIVATE_BYTES: usize = 16 * 1024 * 1024;

fn bounded_kimi_limits(mut limits: ModelConnectorLimits) -> ModelConnectorLimits {
    limits.max_redirects = limits.max_redirects.min(KIMI_MAX_REDIRECTS);
    limits.max_error_body_bytes = limits.max_error_body_bytes.min(KIMI_MAX_ERROR_BODY_BYTES);
    limits.max_sse_event_bytes = limits.max_sse_event_bytes.min(KIMI_MAX_SSE_EVENT_BYTES);
    limits.max_sse_events = limits.max_sse_events.min(KIMI_MAX_SSE_EVENTS);
    limits.max_output_items = limits.max_output_items.min(KIMI_MAX_OUTPUT_ITEMS);
    limits.max_response_text_bytes = limits
        .max_response_text_bytes
        .min(KIMI_MAX_RESPONSE_TEXT_BYTES);
    limits.max_reasoning_bytes = limits.max_reasoning_bytes.min(KIMI_MAX_REASONING_BYTES);
    limits.max_function_argument_bytes = limits
        .max_function_argument_bytes
        .min(KIMI_MAX_FUNCTION_ARGUMENT_BYTES);
    limits.max_provider_private_bytes = limits
        .max_provider_private_bytes
        .min(KIMI_MAX_PROVIDER_PRIVATE_BYTES);
    limits
}

impl SseDecoder for ChatCompletionsSseDecoder {
    fn push(&mut self, bytes: &[u8]) -> DecodeBatch {
        self.push_batch(bytes)
    }
    fn finish(&mut self) -> Result<Vec<yo_core::ModelConnectorEvent>, ConnectorError> {
        self.finish()
    }
}

#[derive(Clone)]
pub struct KimiChatCompletionsConnector {
    client: Client,
    request_url: Url,
    model: String,
    credential: ApiCredential,
    limits: ModelConnectorLimits,
    profile: KimiWireProfile,
}

impl KimiChatCompletionsConnector {
    pub fn new(
        complete: &CompleteModelBinding,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
    ) -> Result<Self, ConnectorError> {
        limits.validate()?;
        let limits = bounded_kimi_limits(limits);
        let profile = request::admit_binding(complete)?;
        let mut request_url = complete
            .binding()
            .endpoint()
            .append_path_segment("chat")
            .map_err(|_| {
                configuration_failure("cannot append chat/completions to the Kimi endpoint")
            })?;
        request_url
            .path_segments_mut()
            .map_err(|_| {
                configuration_failure("cannot append chat/completions to the Kimi endpoint")
            })?
            .push("completions");
        let client = http_client(&request_url, &limits)?;
        Ok(Self {
            client,
            request_url,
            model: complete.binding().model_id().as_str().to_owned(),
            credential,
            limits,
            profile,
        })
    }

    #[must_use]
    pub fn request_url(&self) -> &str {
        self.request_url.as_str()
    }
}

impl ModelConnector for KimiChatCompletionsConnector {
    fn request_url(&self) -> &str {
        self.request_url()
    }

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, ConnectorError> {
        request::wire_body(request, &self.model, self.profile)
    }

    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, ConnectorError> {
        let replay_budget = request.replay_budget().unwrap_or_else(|| {
            yo_core::ModelReplayDelta::replay_budget(None, std::iter::empty())
                .expect("an empty replay delta prefix fits its canonical bound")
        });
        let body = request::wire_body(&request, &self.model, self.profile)?;
        start_stream(
            self.client.clone(),
            self.request_url.clone(),
            self.credential.clone(),
            self.limits.clone(),
            body,
            Box::new(ChatCompletionsSseDecoder::new_kimi_with_replay_budget(
                self.limits.clone(),
                self.model.clone(),
                self.profile.private_replay(),
                replay_budget,
            )),
            cancellation,
            "yo-kimi-chat-completions",
        )
        .map(|stream| {
            Box::new(KimiChatCompletionsStream(stream)) as Box<dyn ModelConnectorStreamPort>
        })
    }
}

trait KimiStream: Send {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), ConnectorError>;
}

impl KimiStream for ConnectorStream {
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

struct KimiChatCompletionsStream<S = ConnectorStream>(S);

impl<S: KimiStream> ModelConnectorStreamPort for KimiChatCompletionsStream<S> {
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

impl fmt::Debug for KimiChatCompletionsConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KimiChatCompletionsConnector")
            .field("request_url", &self.request_url)
            .field("model", &self.model)
            .field("credential", &self.credential)
            .field("limits", &self.limits)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}
