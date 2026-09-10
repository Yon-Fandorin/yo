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
        .with_stderr_diagnostic(stderr_diagnostic)
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

// Stderr may contain cached credentials or user content. Only fixed diagnostics leave
// this backend boundary; matching text and arbitrary paths are never interpolated.
fn stderr_diagnostic(stderr: &str) -> Option<&'static str> {
    if stderr.contains("could not create bwrap placeholder for read-deny path") {
        Some(
            "Grok could not create a bubblewrap placeholder for a denied path and refused a partial sandbox. Check the host bubblewrap/container mount support; the requested sandbox remains required.",
        )
    } else if stderr.contains("could not apply the")
        && stderr.contains("sandbox profile")
        && stderr.contains("Refusing to start with its protections missing")
    {
        Some(
            "Grok could not apply requested sandbox protections; check host sandbox configuration before retrying.",
        )
    } else if stderr.trim().is_empty() {
        None
    } else {
        Some("Grok emitted stderr diagnostics; raw content was withheld to protect credentials.")
    }
}
