use std::{
    error::Error,
    task::{Context, Poll},
};

pub use yo_core::{
    AgentIntent as AgentAction, CommandAdmission as DispatchOutcome,
    PendingCommand as PendingDispatch, SubmissionOutcome,
};
use yo_core::{JournalDurability, RequestTraceEntry, TranscriptRecord};

/// One nonblocking observation exposed to the TUI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentPoll {
    /// No committed record or terminal state is currently available.
    Pending,
    /// One record from the Session Journal's ordered Transcript projection.
    Record(TranscriptRecord),
    /// One payload-free correlation record from the live Session Journal.
    RequestTrace(RequestTraceEntry),
    /// A persistent durability-state transition for the visible Session.
    Durability(JournalDurability),
    /// Whole-request admission resolved for one immutable frontend snapshot.
    Submission(SubmissionOutcome),
    /// The connection closed after exposing every preceding record.
    Closed,
}

/// Frontend-facing connection owned by the product entry point.
///
/// The TUI controls gestures and presentation. Implementations retain Session,
/// Turn, backend, and process policy outside `yo-tui`.
pub trait AgentConnection {
    type Error: Error;

    /// Queues one UI intent without waiting for provider acceptance.
    ///
    /// Committed records and command failures are observed through [`Self::poll`]
    /// so the terminal-owning loop never blocks on provider I/O.
    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error>;

    /// Retries an operation retained by an earlier dispatch attempt.
    fn retry(&mut self, pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error>;

    /// Observes one already committed Transcript record without blocking.
    fn poll(&mut self) -> Result<AgentPoll, Self::Error>;

    /// Registers the frontend task to wake for the next out-of-thread change.
    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()>;
}
