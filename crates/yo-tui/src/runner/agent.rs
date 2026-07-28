use std::error::Error;

use yo_core::RuntimePoll;
pub use yo_core::{
    AgentIntent as AgentAction, CommandAdmission as DispatchOutcome,
    PendingCommand as PendingDispatch,
};

/// Frontend-facing connection owned by the product entry point.
///
/// The TUI controls gestures and presentation. Implementations retain Session,
/// Turn, backend, and process policy outside `yo-tui`.
pub trait AgentConnection {
    type Error: Error;

    /// Queues one UI intent without waiting for provider acceptance.
    ///
    /// Immediate semantic events and command failures are observed through
    /// [`Self::poll`] so the terminal-owning loop never blocks on provider I/O.
    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error>;

    /// Retries an operation retained by an earlier dispatch attempt.
    fn retry(&mut self, pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error>;

    /// Observes one already available semantic event without blocking.
    fn poll(&mut self) -> Result<RuntimePoll, Self::Error>;
}
