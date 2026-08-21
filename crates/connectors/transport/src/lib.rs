//! Bounded Connector HTTP/SSE byte transport with no API-dialect semantics.

use yo_core::{ConnectorError, ModelConnectorEvent};

mod failure;
mod framing;
mod http;
mod stream;
#[cfg(test)]
mod tests;
mod worker;

pub use failure::configuration_failure;
pub use framing::{SseFrame, SseFrameBatch, SseFramer};
pub use http::http_client;
#[cfg(feature = "test-support")]
pub use http::http_client_with_test_root;
pub use stream::ConnectorStream;
pub use worker::start_stream;

const EVENT_QUEUE_CAPACITY: usize = 256;

pub struct DecodeBatch {
    pub events: Vec<ModelConnectorEvent>,
    pub failure: Option<ConnectorError>,
}

pub trait SseDecoder: Send {
    fn push(&mut self, bytes: &[u8]) -> DecodeBatch;
    fn finish(&mut self) -> Result<Vec<ModelConnectorEvent>, ConnectorError>;
}
