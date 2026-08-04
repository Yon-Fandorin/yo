mod error;
mod recovery;
mod state;

pub use error::{AgentRejection, ExpectedResponse, ResponseKind};
use state::EngineState;

use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent,
    SessionId, TurnOutcome, TurnRef, UserInput,
};

/// Deterministic owner of one loaded agent session.
///
/// Every operation still carries explicit session and turn identity. The engine holding one
/// session does not create an implicit "current session" protocol.
#[derive(Debug, Default)]
pub struct AgentEngine {
    state: EngineState,
}

impl AgentEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.state.session_id()
    }

    pub fn active_turn(&self) -> Option<TurnRef> {
        self.state.active_turn()
    }

    pub fn active_turn_input(&self) -> Option<&UserInput> {
        self.state.active_turn_input()
    }

    pub fn turn_count(&self) -> usize {
        self.state.turn_count()
    }

    /// Accepts a frontend command and returns the domain events caused immediately by it.
    ///
    /// Backend work is deliberately absent here. A successful activity response therefore emits
    /// no response Activity yet; a backend adapter will create that correlated Activity.
    pub fn handle_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<Vec<AgentEvent>, AgentRejection> {
        self.state.handle_command(command, false)
    }

    pub(crate) fn validate_command(
        &self,
        command: &AgentCommand,
        supports_steer: bool,
    ) -> Result<(), AgentRejection> {
        self.state.validate_command(command, supports_steer)
    }

    pub(crate) fn commit_command(
        &mut self,
        command: AgentCommand,
        supports_steer: bool,
    ) -> Result<Vec<AgentEvent>, AgentRejection> {
        self.state.handle_command(command, supports_steer)
    }

    pub(crate) fn fail_active_turn(&mut self, failure: crate::Failure) -> Vec<AgentEvent> {
        self.state.fail_active_turn(failure)
    }

    pub(crate) fn interrupt_active_turn(&mut self) -> Vec<AgentEvent> {
        self.state.interrupt_active_turn()
    }

    /// Records the beginning of a backend-produced Activity on the active Turn.
    pub fn start_activity(
        &mut self,
        activity: ActivityRef,
        kind: ActivityKind,
    ) -> Result<AgentEvent, AgentRejection> {
        self.state.start_activity(activity, kind)
    }

    /// Records an incremental update to a currently active Activity.
    pub fn update_activity(
        &mut self,
        activity: ActivityRef,
        update: ActivityUpdate,
    ) -> Result<AgentEvent, AgentRejection> {
        self.state.update_activity(activity, update)
    }

    /// Records the terminal outcome of a currently active Activity.
    pub fn finish_activity(
        &mut self,
        activity: ActivityRef,
        outcome: ActivityOutcome,
    ) -> Result<AgentEvent, AgentRejection> {
        self.state.finish_activity(activity, outcome)
    }

    /// Records a backend-produced terminal outcome for the active Turn.
    pub fn finish_turn(
        &mut self,
        turn: TurnRef,
        outcome: TurnOutcome,
    ) -> Result<AgentEvent, AgentRejection> {
        self.state.finish_turn(turn, outcome)
    }
}
