pub use yo_backend::{
    BackendAdapter, BackendCapabilities, BackendFailure, BackendFailureKind, BackendStopHandle,
};

use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    BackendOutcomeEvidence, BackendRequestEvidence, BackendResumeTarget, ContextCheckpointProposal,
    ContextPolicyChanged, ModelReplayItem, TurnOutcome, TurnRef,
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
    /// A managed exact-replay Session selected or replaced its durable context policy.
    ContextPolicyChanged {
        policy: ContextPolicyChanged,
    },
    /// A tools-disabled summary that awaits exact Journal binding and atomic publication.
    ContextCheckpointPrepared {
        proposal: ContextCheckpointProposal,
    },
    /// A completed active-Turn semantic suffix that core may bind to its exact Journal boundary.
    ContextActiveSuffixCompleted {
        turn: TurnRef,
        items: Vec<ModelReplayItem>,
    },
    /// An internally dispatched ordinary model request awaiting its acceptance record.
    ModelRequestAccepted {
        turn: TurnRef,
        evidence: BackendRequestEvidence,
    },
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
