//! Provider-neutral OpenAI-compatible request transports and bounded SSE decoders.

mod chat_request;
mod chat_sse;
mod connector;
mod framing;
mod request;
mod sse;
mod types;

pub(super) struct SseDecodeBatch {
    pub(super) events: Vec<ResponsesEvent>,
    pub(super) failure: Option<ConnectorError>,
}

pub use connector::{
    OpenAiChatCompletionsConnector, OpenAiResponsesConnector, ResponsesCancellation,
    ResponsesStream,
};
pub use request::{
    FunctionTool, ReasoningEffort, ResponsesInputItem, ResponsesInputRole, ResponsesRequest,
};
pub use types::{
    ConnectorError, ConnectorFailureKind, ReasoningChannel, ResponseTerminal,
    ResponsesConnectorLimits, ResponsesEvent, ResponsesPoll, ResponsesUsage,
};

pub type ModelConnectorCancellation = ResponsesCancellation;
pub type ModelConnectorEvent = ResponsesEvent;
pub type ModelConnectorInputItem = ResponsesInputItem;
pub type ModelConnectorInputRole = ResponsesInputRole;
pub type ModelConnectorLimits = ResponsesConnectorLimits;
pub type ModelConnectorPoll = ResponsesPoll;
pub type ModelConnectorRequest = ResponsesRequest;
pub type ModelConnectorStream = ResponsesStream;
pub type ModelConnectorTerminal = ResponseTerminal;
pub type ModelConnectorUsage = ResponsesUsage;

#[cfg(test)]
pub(crate) mod tests;
