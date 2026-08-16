use serde_json::Value;

use crate::{
    KimiChatCompletionsConnector, ModelConnectorCancellation, ModelConnectorPoll,
    ModelConnectorRequest, ModelConnectorStream, OpenAiChatCompletionsConnector,
    OpenAiResponsesConnector,
};

pub(super) trait ModelConnector: Send {
    fn request_url(&self) -> &str;
    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, crate::ConnectorError>;
    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, crate::ConnectorError>;
}

pub(super) trait ModelConnectorStreamPort: Send {
    fn poll(&mut self) -> Result<ModelConnectorPoll, crate::ConnectorError>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), crate::ConnectorError>;
}

impl ModelConnector for OpenAiResponsesConnector {
    fn request_url(&self) -> &str {
        self.request_url()
    }

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, crate::ConnectorError> {
        Ok(self.tokenization_payload(request))
    }

    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, crate::ConnectorError> {
        OpenAiResponsesConnector::start(self, request, cancellation)
            .map(|stream| Box::new(stream) as Box<dyn ModelConnectorStreamPort>)
    }
}

impl ModelConnector for OpenAiChatCompletionsConnector {
    fn request_url(&self) -> &str {
        self.request_url()
    }

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, crate::ConnectorError> {
        self.tokenization_payload(request)
    }

    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, crate::ConnectorError> {
        OpenAiChatCompletionsConnector::start(self, request, cancellation)
            .map(|stream| Box::new(stream) as Box<dyn ModelConnectorStreamPort>)
    }
}

impl ModelConnector for KimiChatCompletionsConnector {
    fn request_url(&self) -> &str {
        self.request_url()
    }

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<Value, crate::ConnectorError> {
        self.tokenization_payload(request)
    }

    fn start(
        &self,
        request: ModelConnectorRequest,
        cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, crate::ConnectorError> {
        KimiChatCompletionsConnector::start(self, request, cancellation)
            .map(|stream| Box::new(stream) as Box<dyn ModelConnectorStreamPort>)
    }
}

impl ModelConnectorStreamPort for ModelConnectorStream {
    fn poll(&mut self) -> Result<ModelConnectorPoll, crate::ConnectorError> {
        self.poll()
    }

    fn cancel(&self) {
        self.cancel();
    }

    fn shutdown(&mut self) -> Result<(), crate::ConnectorError> {
        self.shutdown()
    }
}
