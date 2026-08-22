//! Provider-neutral Model Connector contract and semantic request/observation types.

mod port;
mod request;
mod types;

pub use port::{ModelConnector, ModelConnectorCancellation, ModelConnectorStreamPort};
#[doc(hidden)]
pub use request::ModelCacheAffinityHint;
pub use request::{
    FunctionTool, ReasoningEffort, RequestToolExposure, ResponsesInputItem, ResponsesInputRole,
    ResponsesRequest,
};
pub use types::{
    CacheReadInputTokens, ConnectorError, ConnectorFailureKind, ReasoningChannel, ResponseTerminal,
    ResponsesConnectorLimits, ResponsesEvent, ResponsesPoll, ResponsesUsage,
};

pub type ModelConnectorEvent = ResponsesEvent;
pub type ModelConnectorInputItem = ResponsesInputItem;
pub type ModelConnectorInputRole = ResponsesInputRole;
pub type ModelConnectorLimits = ResponsesConnectorLimits;
pub type ModelConnectorPoll = ResponsesPoll;
pub type ModelConnectorRequest = ResponsesRequest;
pub type ModelConnectorTerminal = ResponseTerminal;
pub type ModelConnectorUsage = ResponsesUsage;

pub type ResponsesCancellation = ModelConnectorCancellation;

#[cfg(test)]
pub(crate) mod tests;
