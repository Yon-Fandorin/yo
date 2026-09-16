use std::collections::{BTreeMap, BTreeSet};

use super::super::{
    BindingCloseReason, ContextPolicyChanged, ForkItemOrigin, OperationId, VersionedIdentity,
};
use crate::{
    BackendBindingEvidence, ContinuationStrategy, JournalSequence, ModelReplay, ModelReplayItem,
    SessionId, TurnId,
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
    completed_activity_boundaries: BTreeSet<JournalSequence>,
    latest_request_exchange: BTreeMap<(u64, OperationId), JournalSequence>,
    latest_accepted_request: BTreeMap<(u64, TurnId), JournalSequence>,
    completed_turns: BTreeMap<TurnId, JournalSequence>,
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
