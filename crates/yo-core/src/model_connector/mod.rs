//! Provider-neutral OpenAI Responses request transport and bounded SSE decoding.

mod connector;
mod request;
mod sse;
mod types;

pub use connector::{OpenAiResponsesConnector, ResponsesCancellation, ResponsesStream};
pub use request::{
    FunctionTool, ReasoningEffort, ResponsesInputItem, ResponsesInputRole, ResponsesRequest,
};
pub use types::{
    ConnectorError, ConnectorFailureKind, ReasoningChannel, ResponseTerminal,
    ResponsesConnectorLimits, ResponsesEvent, ResponsesPoll, ResponsesUsage,
};

#[cfg(test)]
mod tests;
