//! Provider-neutral Model Connector contract plus the transitional in-core Kimi implementation.

mod chat_sse;
mod connector;
mod framing;
mod kimi_request;
mod port;
mod request;
mod types;

pub(super) struct SseDecodeBatch {
    pub(super) events: Vec<ResponsesEvent>,
    pub(super) failure: Option<ConnectorError>,
}

pub use connector::{KimiChatCompletionsConnector, ResponsesCancellation, ResponsesStream};
pub use port::{ModelConnector, ModelConnectorStreamPort};
pub(crate) use request::ModelCacheAffinityHint;
pub use request::{
    FunctionTool, ReasoningEffort, RequestToolExposure, ResponsesInputItem, ResponsesInputRole,
    ResponsesRequest,
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
