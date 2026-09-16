use std::sync::Arc;

#[cfg(test)]
use serde_json::Value;
use yo_backend::BackendAdapter;
#[cfg(test)]
use yo_core::UserInput;
use yo_core::{
    AgentCommand, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, BackendPoll, BackendResumeTarget,
    BackendStopHandle,
};

use crate::{
    client::AppServerClient,
    config::{CodexBackendConfig, validate_config},
    protocol::CodexCompatibilityWarning,
    transport::StdioPeer,
};

mod events;
mod input;
mod lifecycle;
mod state;
#[cfg(test)]
mod tests;

use state::Backend;

#[cfg(test)]
pub(super) fn project_input(input: &UserInput) -> Result<Vec<Value>, BackendFailure> {
    input::project_input(input)
}
#[cfg(test)]
use state::{InputQuestion, InputQuestions, RequestBinding, RequestKind};

/// Receives Codex compatibility and server warnings without owning process output.
pub type CodexWarningObserver = Arc<dyn Fn(CodexCompatibilityWarning) + Send + Sync + 'static>;

/// Local stdio adapter for a compatible `codex app-server` process.
pub struct CodexBackend {
    inner: Backend<StdioPeer>,
}

impl CodexBackend {
    /// Spawns Codex and prepares the cancellable transport.
    ///
    /// The initialize handshake is deferred to `CreateSession` so the runtime owner can cancel it
    /// through [`yo_core::AgentBackend::stop_handle`].
    pub fn spawn(config: CodexBackendConfig) -> Result<Self, BackendFailure> {
        Self::spawn_with_warning_observer(config, None)
    }

    /// Spawns Codex and forwards compatibility observations to the caller-owned observer.
    pub fn spawn_with_warning_observer(
        config: CodexBackendConfig,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<Self, BackendFailure> {
        validate_config(&config)?;
        let cwd = config
            .working_directory()
            .to_str()
            .ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Initialization,
                    "Codex working directory is not valid UTF-8",
                )
            })?
            .to_owned();
        let peer = StdioPeer::spawn(&config)?;
        let client = AppServerClient::new(peer, config.request_timeout())
            .with_warning_observer(warning_observer);
        let model_rebind_target = config
            .model_rebind_target()
            .map(|(account, model)| (account.clone(), model.clone()));
        let mut inner =
            Backend::new_uninitialized(client, cwd, config.read_only_review(), model_rebind_target);
        inner.new_session_target = config.new_session_target().cloned();
        Ok(Self { inner })
    }

    /// Verifies the local app-server handshake without creating a backend Session.
    pub fn verify(config: CodexBackendConfig) -> Result<(), BackendFailure> {
        Self::verify_with_warning_observer(config, None)
    }

    /// Verifies the local app-server handshake and forwards compatibility observations.
    pub fn verify_with_warning_observer(
        config: CodexBackendConfig,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<(), BackendFailure> {
        let mut backend = Self::spawn_with_warning_observer(config, warning_observer)?;
        let verification = backend.inner.verify();
        let cleanup = backend.inner.shutdown();
        match (verification, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(verification), Ok(())) => Err(verification),
            (Err(verification), Err(cleanup)) => Err(BackendFailure::new(
                verification.kind(),
                format!(
                    "{}; cleanup also failed: {}",
                    verification.message(),
                    cleanup
                ),
            )),
        }
    }
}

impl BackendAdapter for CodexBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        self.inner.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.inner.resume_session(target)
    }

    fn resume_session_rebinding_model(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.inner.resume_session_rebinding_model(target)
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
