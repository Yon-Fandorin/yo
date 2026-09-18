use crate::admission;

mod command;
mod events;
mod input;
mod lifecycle;
mod state;

#[cfg(test)]
mod tests;

use state::Backend;
use yo_backend::BackendAdapter;
use yo_core::{
    AgentCommand, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, BackendPoll, BackendResumeTarget,
    BackendStopHandle,
};

use crate::{
    client::{AcpClient, combine_with_cleanup},
    config::GrokBackendConfig,
    transport::{JsonPeer, StdioPeer},
};

/// Local stdio adapter for the Grok Build Agent Client Protocol service.
pub struct GrokBackend {
    inner: Backend<StdioPeer>,
}

impl GrokBackend {
    /// Spawns `grok agent stdio`; initialization and cached-token authentication are deferred.
    pub fn spawn(config: GrokBackendConfig) -> Result<Self, BackendFailure> {
        admission::validate_config(&config)?;
        let cwd = config
            .working_directory()
            .to_str()
            .ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Initialization,
                    "Grok working directory is not valid UTF-8",
                )
            })?
            .to_owned();
        let peer = StdioPeer::spawn(&config)?;
        let client = AcpClient::new(peer, config.request_timeout());
        Ok(Self {
            inner: Backend::new_uninitialized(client, cwd, config.read_only_review()),
        })
    }

    /// Verifies ACP compatibility and the cached Grok login without creating a Session.
    pub fn verify(config: GrokBackendConfig) -> Result<(), BackendFailure> {
        let mut backend = Self::spawn(config)?;
        let verification = backend.inner.verify();
        let cleanup = backend.inner.shutdown();
        combine_with_cleanup(verification, cleanup)
    }
}

impl BackendAdapter for GrokBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        self.inner.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.inner.resume_session(target)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.inner.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.inner.poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.inner.shutdown()
    }
}

impl<P: JsonPeer> BackendAdapter for Backend<P> {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        self.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_session(target)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.shutdown()
    }
}
