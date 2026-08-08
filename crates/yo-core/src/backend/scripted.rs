use std::collections::VecDeque;

use super::{
    AgentBackend, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, BackendPoll, BackendResumeTarget,
    BackendStopHandle,
};
use crate::AgentCommand;

/// One deterministic expectation or observation in a [`ScriptedBackend`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendScriptStep {
    Resume {
        target: Box<BackendResumeTarget>,
        evidence: BackendBindingEvidence,
    },
    AcceptCommand(AgentCommand),
    AcceptCommandWithEvidence {
        command: AgentCommand,
        evidence: BackendCommandEvidence,
    },
    RejectCommand {
        command: AgentCommand,
        failure: BackendFailure,
    },
    Emit(BackendEvent),
    Fail(BackendFailure),
    Close,
    Shutdown(Result<(), BackendFailure>),
}

/// Deterministic backend used to exercise agent flows without a provider process.
#[derive(Debug)]
pub struct ScriptedBackend {
    capabilities: BackendCapabilities,
    steps: VecDeque<BackendScriptStep>,
    closed: bool,
    shutdown_result: Option<Result<(), BackendFailure>>,
}

impl ScriptedBackend {
    pub fn new(steps: impl IntoIterator<Item = BackendScriptStep>) -> Self {
        Self {
            capabilities: BackendCapabilities::default(),
            steps: steps.into_iter().collect(),
            closed: false,
            shutdown_result: None,
        }
    }

    pub fn with_capabilities(mut self, capabilities: BackendCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn remaining_steps(&self) -> usize {
        self.steps.len()
    }

    pub fn is_exhausted(&self) -> bool {
        self.steps.is_empty()
    }

    fn protocol_failure(message: impl Into<String>) -> BackendFailure {
        BackendFailure::new(BackendFailureKind::Protocol, message)
    }
}

impl AgentBackend for ScriptedBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        BackendStopHandle::no_op()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        match self.steps.front() {
            Some(BackendScriptStep::Resume {
                target: expected, ..
            }) if expected.as_ref() == target => {
                let Some(BackendScriptStep::Resume { evidence, .. }) = self.steps.pop_front()
                else {
                    unreachable!("the front script step was native resume")
                };
                Ok(evidence)
            },
            Some(step) => Err(Self::protocol_failure(format!(
                "unexpected native resume while awaiting {step:?}"
            ))),
            None => Err(Self::protocol_failure(
                "unexpected native resume after the script was exhausted",
            )),
        }
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.shutdown_result.is_some() || self.closed {
            return Err(Self::protocol_failure(
                "cannot execute a command after the backend closed",
            ));
        }

        match self.steps.front() {
            Some(BackendScriptStep::AcceptCommand(expected)) if expected == &command => {
                self.steps.pop_front();
                Ok(BackendCommandEvidence::None)
            },
            Some(BackendScriptStep::AcceptCommandWithEvidence {
                command: expected, ..
            }) if expected == &command => {
                let Some(BackendScriptStep::AcceptCommandWithEvidence { evidence, .. }) =
                    self.steps.pop_front()
                else {
                    unreachable!("the front script step carried command evidence");
                };
                Ok(evidence)
            },
            Some(BackendScriptStep::RejectCommand {
                command: expected, ..
            }) if expected == &command => {
                let Some(BackendScriptStep::RejectCommand { failure, .. }) = self.steps.pop_front()
                else {
                    unreachable!("the front script step was a command rejection");
                };
                Err(failure)
            },
            Some(
                BackendScriptStep::AcceptCommand(expected)
                | BackendScriptStep::AcceptCommandWithEvidence {
                    command: expected, ..
                }
                | BackendScriptStep::RejectCommand {
                    command: expected, ..
                },
            ) => Err(Self::protocol_failure(format!(
                "unexpected command: expected {expected:?}, received {command:?}"
            ))),
            Some(step) => Err(Self::protocol_failure(format!(
                "unexpected command while awaiting {step:?}"
            ))),
            None => Err(Self::protocol_failure(
                "unexpected command after the script was exhausted",
            )),
        }
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if self.shutdown_result.is_some() || self.closed {
            return Ok(BackendPoll::Closed);
        }

        match self.steps.front() {
            Some(BackendScriptStep::Emit(_)) => {
                let Some(BackendScriptStep::Emit(event)) = self.steps.pop_front() else {
                    unreachable!("the front script step was an event");
                };
                Ok(BackendPoll::Event(event))
            },
            Some(BackendScriptStep::Fail(_)) => {
                let Some(BackendScriptStep::Fail(failure)) = self.steps.pop_front() else {
                    unreachable!("the front script step was a failure");
                };
                Err(failure)
            },
            Some(BackendScriptStep::Close) => {
                self.steps.pop_front();
                self.closed = true;
                Ok(BackendPoll::Closed)
            },
            Some(
                BackendScriptStep::Resume { .. }
                | BackendScriptStep::AcceptCommand(_)
                | BackendScriptStep::AcceptCommandWithEvidence { .. }
                | BackendScriptStep::RejectCommand { .. }
                | BackendScriptStep::Shutdown(_),
            )
            | None => Ok(BackendPoll::Pending),
        }
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        if let Some(result) = &self.shutdown_result {
            return result.clone();
        }

        let result = match self.steps.front() {
            Some(BackendScriptStep::Shutdown(_)) => {
                let Some(BackendScriptStep::Shutdown(result)) = self.steps.pop_front() else {
                    unreachable!("the front script step was a shutdown result");
                };
                result
            },
            Some(step) => Err(Self::protocol_failure(format!(
                "shutdown occurred while awaiting {step:?}"
            ))),
            None => Err(Self::protocol_failure(
                "shutdown was not declared by the script",
            )),
        };
        self.shutdown_result = Some(result.clone());
        self.closed = true;
        result
    }
}
