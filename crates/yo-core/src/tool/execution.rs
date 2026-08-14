use std::time::Duration;

use super::{errors::ToolExecutionError, registry::ValidatedToolCall, schema::ToolId};
use crate::TurnRef;

#[derive(Clone, Debug)]
pub struct ToolExecutionRequest {
    pub turn: TurnRef,
    pub call: ValidatedToolCall,
    pub maximum_output_bytes: usize,
    pub absolute_execution_timeout: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionOutcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionResult {
    outcome: ToolExecutionOutcome,
    output: String,
    truncated: bool,
}

impl ToolExecutionResult {
    pub fn new(outcome: ToolExecutionOutcome, output: impl Into<String>, truncated: bool) -> Self {
        Self {
            outcome,
            output: output.into(),
            truncated,
        }
    }

    pub const fn outcome(&self) -> ToolExecutionOutcome {
        self.outcome
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionPoll {
    Pending,
    Ready,
}

pub trait ToolExecution: Send {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError>;
    fn take_result(&mut self) -> Option<ToolExecutionResult>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), ToolExecutionError>;
}

pub trait ToolExecutionHost: Send {
    fn identity(&self) -> &str;
    fn is_available(&self, tool: &ToolId) -> bool;
    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError>;
    fn shutdown(&mut self) -> Result<(), ToolExecutionError>;
}
