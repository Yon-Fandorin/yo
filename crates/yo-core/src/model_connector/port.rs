use std::fmt;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{ConnectorError, ModelConnectorPoll, ModelConnectorRequest};

#[derive(Clone, Default)]
pub struct ModelConnectorCancellation(CancellationToken);

impl ModelConnectorCancellation {
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

impl fmt::Debug for ModelConnectorCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelConnectorCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// One concrete API-dialect connector injected by the process composition root.
pub trait ModelConnector: Send {
    fn request_url(&self) -> &str;

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, ConnectorError>;

    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, ConnectorError>;
}

/// Polling and cleanup boundary for one concrete connector request.
pub trait ModelConnectorStreamPort: Send {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), ConnectorError>;
}
