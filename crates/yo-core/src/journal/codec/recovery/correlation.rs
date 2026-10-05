use std::collections::{BTreeMap, BTreeSet};

use super::super::{
    BindingCloseReason, ContextPolicyChanged, ForkItemOrigin, OperationId, VersionedIdentity,
};
use crate::{
    ActivityKind, ActivityQuestion, ActivityRef, ActivityRequestRef, ActivityResponse,
    BackendBindingEvidence, ContinuationStrategy, JournalSequence, ModelReplay, ModelReplayItem,
    RequestId, SessionId, TurnId,
};

mod binding;
mod context;
mod model;
mod observe;
mod origins;

#[cfg(test)]
mod tests;

use model::{ReferenceTarget, RegisteredFork, ReplacementSource, ReplayDeltaSource, ReplayGroup};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CorrelationRecovery {
    reference_targets: BTreeMap<JournalSequence, ReferenceTarget>,
    operation_roots: BTreeSet<OperationId>,
    submission_commands: BTreeMap<OperationId, TurnId>,
    active_turn_starts: BTreeMap<TurnId, JournalSequence>,
    submitted_inputs: BTreeMap<JournalSequence, ModelReplayItem>,
    submitted_input_turns: BTreeMap<JournalSequence, TurnId>,
    closed_activity_boundaries: BTreeMap<JournalSequence, (TurnId, ActivityKind)>,
    interrupted_activity_boundaries: BTreeMap<JournalSequence, TurnId>,
    started_activities: BTreeMap<ActivityRef, (JournalSequence, ActivityKind)>,
    completed_activity_boundaries: BTreeMap<JournalSequence, (TurnId, ActivityKind)>,
    question_text: BTreeMap<ActivityRef, Option<String>>,
    ordinary_questions: BTreeMap<ActivityRequestRef, ActivityQuestion>,
    answered_questions: BTreeMap<ActivityRequestRef, (ActivityQuestion, ActivityResponse)>,
    completed_questions: BTreeMap<(TurnId, RequestId), (ActivityQuestion, ActivityResponse)>,
    question_response_boundaries:
        BTreeMap<JournalSequence, (TurnId, ActivityQuestion, ActivityResponse)>,
    latest_request_exchange: BTreeMap<(u64, OperationId), JournalSequence>,
    latest_accepted_request: BTreeMap<(u64, TurnId), JournalSequence>,
    completed_turns: BTreeMap<TurnId, JournalSequence>,
    failed_turns: BTreeMap<TurnId, (JournalSequence, JournalSequence)>,
    accepted_request_history: BTreeMap<JournalSequence, (u64, TurnId)>,
    secret_submission_barrier: bool,
    active_checkpoint_boundaries: BTreeMap<JournalSequence, (TurnId, JournalSequence)>,
    session_id: Option<SessionId>,
    session_created: bool,
    open_epoch: Option<u64>,
    open_strategy: Option<ContinuationStrategy>,
    last_epoch: Option<u64>,
    last_close_reason: Option<BindingCloseReason>,
    replacement_source: Option<ReplacementSource>,
    replacement_without_source_allowed: bool,
    open_epoch_has_accepted_request: bool,
    latest_anchor: Option<JournalSequence>,
    latest_checkpoint: Option<JournalSequence>,
    request_after_checkpoint: bool,
    context_epoch: Option<u64>,
    current_policy: Option<ContextPolicyChanged>,
    saw_legacy_context_record: bool,
    record_coordinates: BTreeMap<JournalSequence, (u64, u64)>,
    replay_deltas: BTreeMap<JournalSequence, ReplayDeltaSource>,
    replay_groups: Vec<ReplayGroup>,
    open_binding_identity: Option<VersionedIdentity>,
    replay_contract_rebind_required: bool,
    model_replay: ModelReplay,
    model_replay_origins: Vec<ForkItemOrigin>,
    open_binding: Option<BackendBindingEvidence>,
    fork_seed: Option<RegisteredFork>,
}
