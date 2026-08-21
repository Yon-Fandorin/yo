use std::fmt;

use failure::configuration_failure;
use reqwest::{Client, Url};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{
    ConnectorError, ResponsesConnectorLimits, ResponsesEvent, ResponsesRequest, SseDecodeBatch,
    chat_sse::ChatCompletionsSseDecoder,
    kimi_request::{self, KimiWireProfile},
};
use crate::{ApiCredential, CompleteModelBinding};

const EVENT_QUEUE_CAPACITY: usize = 256;

mod failure;
mod stream;
mod transport;
mod worker;

#[cfg(test)]
mod tests;

pub use stream::ResponsesStream;

trait SseDecoder: Send {
    fn push(&mut self, bytes: &[u8]) -> SseDecodeBatch;
    fn finish(&mut self) -> Result<Vec<ResponsesEvent>, ConnectorError>;
}

impl SseDecoder for ChatCompletionsSseDecoder {
    fn push(&mut self, bytes: &[u8]) -> SseDecodeBatch {
        self.push_batch(bytes)
    }

    fn finish(&mut self) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        self.finish()
    }
}

#[derive(Clone, Default)]
pub struct ResponsesCancellation(CancellationToken);

impl ResponsesCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }
}

impl fmt::Debug for ResponsesCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponsesCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone)]
pub struct KimiChatCompletionsConnector {
    client: Client,
    request_url: Url,
    model: String,
    credential: ApiCredential,
    limits: ResponsesConnectorLimits,
    profile: KimiWireProfile,
}

impl KimiChatCompletionsConnector {
    pub fn new(
        complete: &CompleteModelBinding,
        credential: ApiCredential,
        limits: ResponsesConnectorLimits,
    ) -> Result<Self, ConnectorError> {
        limits.validate()?;
        let profile = kimi_request::admit_binding(complete)?;
        let request_url = complete
            .binding()
            .endpoint()
            .append_path_segments(&["chat", "completions"])
            .map_err(|_| {
                configuration_failure("cannot append chat/completions to the Kimi endpoint")
            })?;
        let client = transport::http_client(&request_url, &limits)?;
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

    pub fn tokenization_payload(
        &self,
        request: &ResponsesRequest,
    ) -> Result<Value, ConnectorError> {
        kimi_request::wire_body(request, &self.model, self.profile)
    }

    pub fn start(
        &self,
        request: ResponsesRequest,
        cancellation: ResponsesCancellation,
    ) -> Result<ResponsesStream, ConnectorError> {
        let replay_budget = request.replay_budget().unwrap_or_else(|| {
            crate::ModelReplayDelta::replay_budget(None, std::iter::empty())
                .expect("an empty replay delta prefix fits its canonical bound")
        });
        let body = kimi_request::wire_body(&request, &self.model, self.profile)?;
        worker::start_stream(
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
