use std::time::Duration;

use super::{errors::ToolExecutionError, registry::ValidatedToolCall, schema::ToolId};
use crate::TurnRef;

#[derive(Clone, Debug)]
pub struct ToolExecutionRequest {
    pub turn: TurnRef,
    pub call: ValidatedToolCall,
    pub maximum_output_bytes: usize,
    /// Separate, optional text retention budget; never increases model replay output.
    pub maximum_retained_output_bytes: Option<usize>,
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
    retained_output: Option<(String, bool)>,
}

impl ToolExecutionResult {
    pub fn new(outcome: ToolExecutionOutcome, output: impl Into<String>, truncated: bool) -> Self {
        Self {
            outcome,
            output: output.into(),
            truncated,
            retained_output: None,
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

    /// Host-produced retained text, still awaiting semantic admission and storage.
    /// `truncated` describes this text independently of the model-facing result.
    pub fn with_retained_output(mut self, output: impl Into<String>, truncated: bool) -> Self {
        self.retained_output = Some((output.into(), truncated));
        self
    }

    /// Optional retained text and its omission observation.
    pub fn retained_output(&self) -> Option<(&str, bool)> {
        self.retained_output
            .as_ref()
            .map(|(text, truncated)| (text.as_str(), *truncated))
    }
}

/// Latest bounded, nonterminal output snapshot. It is never a replay result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionProgress {
    /// Host-produced snapshot, awaiting semantic admission.
    pub output: String,
    /// Whether the host omitted any output bytes.
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionPoll {
    Pending,
    Ready,
}

pub trait ToolExecution: Send {
    /// Takes the newest coalesced progress snapshot, if this host supports streaming.
    fn take_progress(&mut self) -> Option<ToolExecutionProgress> {
        None
    }

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
