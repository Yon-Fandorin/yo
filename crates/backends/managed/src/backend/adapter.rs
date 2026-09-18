//! 수명 주기, 재개, 명령, 폴링 작업을 위한 백엔드 어댑터 파사드.

mod commands;
mod lifecycle;
mod poll;
mod resume;

use yo_backend::BackendAdapter;
use yo_core::{
    AgentCommand, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendPoll, BackendResumeTarget, BackendStopHandle,
};

use super::NativeModelBackend;

impl BackendAdapter for NativeModelBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        lifecycle::stop_handle(self)
    }

    fn capabilities(&self) -> BackendCapabilities {
        lifecycle::capabilities(self)
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        resume::resume_session(self, target)
    }

    fn resume_session_replacing_binding(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        resume::resume_session_replacing_binding(self, target)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        commands::execute_command(self, command)
    }

    fn commit_prepared_command(&mut self) -> Result<(), BackendFailure> {
        self.commit_secret_request()
    }

    fn abort_prepared_command(&mut self) -> Result<(), BackendFailure> {
        self.abort_secret_request();
        Ok(())
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        poll::poll_event(self)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        lifecycle::shutdown(self)
    }
}
