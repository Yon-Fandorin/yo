pub use yo_backend::{
    BackendAdapter, BackendCapabilities, BackendFailure, BackendFailureKind, BackendStopHandle,
};

use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    BackendOutcomeEvidence, BackendResumeTarget, TurnOutcome, TurnRef,
};

/// Yo's frontend-independent specialization of the generic backend adapter port.
pub trait AgentBackend:
    BackendAdapter<Command = AgentCommand, Event = BackendEvent, ResumeTarget = BackendResumeTarget>
{
}

impl<T> AgentBackend for T where
    T: BackendAdapter<
            Command = AgentCommand,
            Event = BackendEvent,
            ResumeTarget = BackendResumeTarget,
        > + ?Sized
{
}

pub type BackendPoll = yo_backend::BackendPoll<BackendEvent>;

/// Semantic output a backend may submit to `yo-core`.
///
/// Session and Turn creation remain core command transitions, so a backend cannot fabricate their
/// corresponding frontend events through this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendEvent {
    ActivityStarted {
        activity: ActivityRef,
        kind: ActivityKind,
    },
    ActivityUpdated {
        activity: ActivityRef,
        update: ActivityUpdate,
    },
    ActivityFinished {
        activity: ActivityRef,
        outcome: ActivityOutcome,
    },
    TurnFinished {
        turn: TurnRef,
        outcome: TurnOutcome,
    },
    /// A completed Turn whose backend binding can continue from the accepted request.
    ResumableTurnFinished {
        turn: TurnRef,
        evidence: BackendOutcomeEvidence,
    },
}
