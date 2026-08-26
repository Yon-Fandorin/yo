use std::time::Duration;

use serde_json::Value;
pub(super) use yo_backend::transport::JsonlPoll as PeerPoll;
use yo_backend::{
    BackendFailure, BackendStopHandle,
    transport::{JsonMessagePeer, StdioJsonlConfig, StdioJsonlPeer},
};

use super::GrokBackendConfig;

pub(super) trait JsonPeer: JsonMessagePeer {}

impl<T: JsonMessagePeer> JsonPeer for T {}

pub(super) struct StdioPeer(StdioJsonlPeer);

impl StdioPeer {
    pub(super) fn spawn(config: &GrokBackendConfig) -> Result<Self, BackendFailure> {
        let transport = StdioJsonlConfig::new(
            "Grok Build ACP agent",
            "grok",
            config.executable(),
            config.working_directory(),
        )
        .with_arguments(config.process_arguments())
        .with_shutdown_timeout(config.shutdown_timeout());
        StdioJsonlPeer::spawn(transport).map(Self)
    }
}

impl JsonMessagePeer for StdioPeer {
    fn stop_handle(&self) -> BackendStopHandle {
        self.0.stop_handle()
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        self.0.send(message)
    }

    fn receive(&mut self, timeout: Duration) -> Result<PeerPoll, BackendFailure> {
        self.0.receive(timeout)
    }

    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
        self.0.try_receive()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.0.shutdown()
    }
}
