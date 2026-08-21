use serde_json::Value;

use super::{
    ConnectorError, KimiChatCompletionsConnector, ModelConnectorCancellation, ModelConnectorPoll,
    ModelConnectorRequest, ModelConnectorStream,
};

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

impl ModelConnector for KimiChatCompletionsConnector {
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
        KimiChatCompletionsConnector::start(self, request, cancellation)
            .map(|stream| Box::new(stream) as Box<dyn ModelConnectorStreamPort>)
    }
}

impl ModelConnectorStreamPort for ModelConnectorStream {
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
