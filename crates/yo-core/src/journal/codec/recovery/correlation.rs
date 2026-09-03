use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
};

use sha2::{Digest, Sha256};

use super::super::{
    BindingCloseReason, CacheState, ContextLoss, ContextPolicyChanged, ContextStrategy,
    ExchangeDirection, ExchangeKind, JournalCodecError, JournalRecord, OperationId, TransitionMode,
    VersionedIdentity,
};
use crate::{
    AgentCommand, AgentEvent, ContinuationStrategy, JournalSequence, ModelReplay, ModelReplayItem,
    ReplayProfile, SessionId, TurnId, TurnOutcome,
    backend::{provider_private_schema, validate_provider_private_replay_sequence},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CorrelationRecovery {
    reference_targets: BTreeMap<JournalSequence, ReferenceTarget>,
    operation_roots: BTreeSet<OperationId>,
    submission_commands: BTreeMap<OperationId, TurnId>,
    active_turn_starts: BTreeMap<TurnId, JournalSequence>,
    submitted_inputs: BTreeMap<JournalSequence, String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // source range 검증은 숫자 공간을 순회하지 않고 실제 Journal 좌표만 조회해야 하므로
    // 극단적으로 먼 두 sequence도 bounded record 수에 비례해 처리됩니다.
    #[test]
    fn source_range_validation_is_bounded_by_present_records() {
        let first = JournalSequence::new(1);
        let last = JournalSequence::new(u64::MAX - 1);
        let mut recovery = CorrelationRecovery::default();
        recovery.record_coordinates.insert(first, (7, 9));
        recovery.record_coordinates.insert(last, (7, 9));

        recovery
            .validate_source_range(first, last, (7, 9), "sparse source range")
            .unwrap();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReferenceTarget {
    Exchange(IndexedExchange),
    Anchor {
        epoch: u64,
        context_epoch: Option<u64>,
        journal_boundary: JournalSequence,
    },
    Checkpoint {
        epoch: u64,
        context_epoch: u64,
        binding_identity: VersionedIdentity,
        replay_profile: ReplayProfile,
        has_provider_private: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IndexedExchange {
    epoch: u64,
    operation_id: OperationId,
    kind: ExchangeKind,
    direction: ExchangeDirection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayDeltaSource {
    epoch: u64,
    context_epoch: u64,
    turn_id: TurnId,
    accepted_request_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayGroup {
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    replay_delta_sequence: JournalSequence,
    epoch: u64,
    context_epoch: u64,
    items: Vec<ModelReplayItem>,
}

fn artifact_matches(
    item: &ModelReplayItem,
    receipt: &super::super::ContextArtifactReceipt,
) -> bool {
    if receipt.media_kind() != "text/plain" {
        return false;
    }
    let matches = |bytes: &[u8]| {
        let digest = Sha256::digest(bytes);
        let mut content_hash = String::from("sha256:");
        for byte in digest {
            write!(&mut content_hash, "{byte:02x}")
                .expect("writing a digest into a String cannot fail");
        }
        u64::try_from(bytes.len()) == Ok(receipt.byte_count())
            && content_hash == receipt.content_hash()
    };
    match item {
        ModelReplayItem::FunctionCallOutput { output, .. } => matches(output.as_bytes()),
        ModelReplayItem::Message { .. }
        | ModelReplayItem::FunctionCall { .. }
        | ModelReplayItem::ProviderPrivateAssistant { .. } => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplacementSource {
    Anchor {
        sequence: JournalSequence,
        epoch: u64,
    },
    Checkpoint {
        sequence: JournalSequence,
        epoch: u64,
    },
}

impl From<&super::super::BackendExchangeObserved> for IndexedExchange {
    fn from(exchange: &super::super::BackendExchangeObserved) -> Self {
        Self {
            epoch: exchange.epoch(),
            operation_id: exchange.operation_id(),
            kind: exchange.kind(),
            direction: exchange.direction(),
        }
    }
}

impl CorrelationRecovery {
    pub(super) fn observe(
        &mut self,
        sequence: JournalSequence,
        record: &JournalRecord,
        previous_in_commit: Option<(JournalSequence, &JournalRecord)>,
    ) -> Result<(), JournalCodecError> {
        let anchor_before_record = self.latest_anchor;
        if !matches!(
            record,
            JournalRecord::ContinuationAnchor(_) | JournalRecord::ContextPolicyChanged(_)
        ) {
            self.latest_anchor = None;
        }

        match record {
            JournalRecord::CommandCommitted(command) => self.observe_command(sequence, command),
            JournalRecord::EventCommitted(event) => self.observe_event(sequence, event),
            JournalRecord::BackendExchangeObserved(exchange) => {
                self.observe_exchange(sequence, exchange)?;
            },
            JournalRecord::BackendBindingOpened(binding) => {
                self.observe_binding_open(sequence, binding)?;
            },
            JournalRecord::BackendBindingClosed(binding) => {
                if self.open_epoch != Some(binding.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_binding_closed must close the current epoch",
                    ));
                }
                self.open_epoch = None;
                self.open_strategy = None;
                self.open_binding_identity = None;
                self.last_close_reason = Some(binding.reason());
                self.replacement_without_source_allowed = binding.reason()
                    == BindingCloseReason::Replaced
                    && !self.open_epoch_has_accepted_request;
                self.replacement_source = if binding.reason() == BindingCloseReason::Replaced {
                    anchor_before_record
                        .map(|sequence| ReplacementSource::Anchor {
                            sequence,
                            epoch: binding.epoch(),
                        })
                        .or_else(|| {
                            (!self.request_after_checkpoint)
                                .then_some(self.latest_checkpoint)
                                .flatten()
                                .map(|sequence| ReplacementSource::Checkpoint {
                                    sequence,
                                    epoch: binding.epoch(),
                                })
                        })
                } else {
                    None
                };
            },
            JournalRecord::BackendRequestAccepted(request) => {
                self.observe_context_epoch(request.context_epoch(), "backend_request_accepted")?;
                if self.open_epoch != Some(request.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted requires its verified open epoch",
                    ));
                }
                let operation = request.operation_id();
                if let Some(command_turn) = self.submission_commands.get(&operation) {
                    if *command_turn != request.turn_id() {
                        return Err(JournalCodecError::new(
                            "backend_request_accepted turn does not match its submission command",
                        ));
                    }
                } else {
                    let is_internal_successor = self.session_id.is_some_and(|session_id| {
                        let has_prior_request = self
                            .latest_accepted_request
                            .contains_key(&(request.epoch(), request.turn_id()));
                        let is_first_post_checkpoint_request = self
                            .latest_checkpoint
                            .zip(self.active_turn_starts.get(&request.turn_id()).copied())
                            .is_some_and(|(checkpoint, start)| {
                                start < checkpoint && !self.request_after_checkpoint
                            });
                        self.active_turn_starts.contains_key(&request.turn_id())
                            && (has_prior_request || is_first_post_checkpoint_request)
                            && operation
                                == OperationId::for_internal_request(
                                    session_id,
                                    request.turn_id(),
                                    request.exchange_sequence(),
                                )
                    });
                    if !is_internal_successor {
                        return Err(JournalCodecError::new(
                            "backend_request_accepted has neither a matching submission command nor a valid writer-assigned successor identity",
                        ));
                    }
                }
                if self
                    .latest_request_exchange
                    .get(&(request.epoch(), operation))
                    != Some(&request.exchange_sequence())
                {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted must reference the latest outbound request exchange",
                    ));
                }
                self.latest_accepted_request
                    .insert((request.epoch(), request.turn_id()), sequence);
                self.open_epoch_has_accepted_request = true;
                if self.latest_checkpoint.is_some() {
                    self.request_after_checkpoint = true;
                }
            },
            JournalRecord::ModelReplayDelta(replay) => {
                self.observe_context_epoch(replay.context_epoch(), "model_replay_delta")?;
                if self.open_epoch != Some(replay.epoch())
                    || !matches!(
                        self.open_strategy,
                        Some(ContinuationStrategy::ExactReplay { .. })
                    )
                {
                    return Err(JournalCodecError::new(
                        "model_replay_delta requires an exact-replay open epoch",
                    ));
                }
                let has_provider_private =
                    replay.delta().items().iter().any(|item| {
                        matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. })
                    });
                match self.open_strategy {
                    Some(ContinuationStrategy::ExactReplay {
                        replay_profile: ReplayProfile::SemanticOnly,
                        ..
                    }) if has_provider_private => {
                        return Err(JournalCodecError::new(
                            "semantic-only exact replay cannot contain provider-private items",
                        ));
                    },
                    Some(ContinuationStrategy::ExactReplay {
                        replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                        ..
                    }) => {
                        validate_provider_private_replay_sequence(
                            replay.delta().items(),
                            provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext)
                                .expect("the provider-private profile has an exact schema"),
                        )
                        .map_err(JournalCodecError::new)?;
                    },
                    _ => {},
                }
                if self
                    .latest_accepted_request
                    .get(&(replay.epoch(), replay.turn_id()))
                    != Some(&replay.accepted_request_sequence())
                {
                    return Err(JournalCodecError::new(
                        "model_replay_delta must reference the latest accepted request",
                    ));
                }
                let Some((
                    _,
                    JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                        turn,
                        outcome: TurnOutcome::Completed,
                    }),
                )) = previous_in_commit
                else {
                    return Err(JournalCodecError::new(
                        "model_replay_delta must immediately follow its completed Turn",
                    ));
                };
                if turn.turn_id() != replay.turn_id() {
                    return Err(JournalCodecError::new(
                        "model_replay_delta Turn does not match its completed Turn",
                    ));
                }
                let apply_result = if self.replay_contract_rebind_required {
                    self.model_replay.apply_binding_replacement(replay.delta())
                } else {
                    self.model_replay.apply(replay.delta())
                };
                apply_result.map_err(|detail| {
                    JournalCodecError::new(format!(
                        "model_replay_delta cannot extend the replay chain: {detail}"
                    ))
                })?;
                self.replay_contract_rebind_required = false;
                if let Some(context_epoch) = replay.context_epoch() {
                    self.replay_deltas.insert(
                        sequence,
                        ReplayDeltaSource {
                            epoch: replay.epoch(),
                            context_epoch,
                            turn_id: replay.turn_id(),
                            accepted_request_sequence: replay.accepted_request_sequence(),
                            items: replay.delta().items().to_vec(),
                        },
                    );
                }
            },
            JournalRecord::BackendResumableOutcome(outcome) => {
                self.observe_context_epoch(outcome.context_epoch(), "backend_resumable_outcome")?;
                if self.open_epoch != Some(outcome.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_resumable_outcome requires its open epoch",
                    ));
                }
                if self
                    .latest_accepted_request
                    .get(&(outcome.epoch(), outcome.turn_id()))
                    != Some(&outcome.accepted_request_sequence())
                {
                    return Err(JournalCodecError::new(
                        "backend_resumable_outcome must reference the latest accepted request",
                    ));
                }
                if !self
                    .completed_turns
                    .get(&outcome.turn_id())
                    .is_some_and(|completed| *completed > outcome.accepted_request_sequence())
                {
                    return Err(JournalCodecError::new(
                        "backend_resumable_outcome requires a preceding completed Turn",
                    ));
                }
                match self.open_strategy {
                    Some(ContinuationStrategy::ExactReplay { .. }) => {
                        let Some(replay_sequence) = outcome.replay_delta_sequence() else {
                            return Err(JournalCodecError::new(
                                "exact-replay outcome requires replay_delta_sequence",
                            ));
                        };
                        let Some((previous_sequence, JournalRecord::ModelReplayDelta(replay))) =
                            previous_in_commit
                        else {
                            return Err(JournalCodecError::new(
                                "exact-replay outcome must immediately follow its replay delta",
                            ));
                        };
                        if replay_sequence != previous_sequence
                            || replay.epoch() != outcome.epoch()
                            || replay.turn_id() != outcome.turn_id()
                            || replay.accepted_request_sequence()
                                != outcome.accepted_request_sequence()
                        {
                            return Err(JournalCodecError::new(
                                "exact-replay outcome does not match its replay delta",
                            ));
                        }
                    },
                    Some(ContinuationStrategy::BackendManagedState) => {
                        if outcome.replay_delta_sequence().is_some()
                            || !matches!(
                                previous_in_commit,
                                Some((_, JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                                    turn,
                                    outcome: TurnOutcome::Completed,
                                }))) if turn.turn_id() == outcome.turn_id()
                            )
                        {
                            return Err(JournalCodecError::new(
                                "backend-managed outcome must immediately follow its completed Turn without replay evidence",
                            ));
                        }
                    },
                    None => {
                        return Err(JournalCodecError::new(
                            "backend_resumable_outcome requires a continuation strategy",
                        ));
                    },
                }
            },
            JournalRecord::ContinuationAnchor(anchor) => {
                self.observe_context_epoch(anchor.context_epoch(), "continuation_anchor")?;
                let Some((previous_sequence, JournalRecord::BackendResumableOutcome(outcome))) =
                    previous_in_commit
                else {
                    return Err(JournalCodecError::new(
                        "continuation_anchor must immediately follow its resumable outcome in one commit",
                    ));
                };
                if previous_sequence != anchor.resumable_outcome_sequence()
                    || anchor.journal_boundary() != previous_sequence
                    || anchor.epoch() != outcome.epoch()
                    || anchor.accepted_request_sequence() != outcome.accepted_request_sequence()
                {
                    return Err(JournalCodecError::new(
                        "continuation_anchor does not match its resumable outcome",
                    ));
                }
                if let (Some(context_epoch), Some(replay_delta_sequence)) =
                    (anchor.context_epoch(), outcome.replay_delta_sequence())
                {
                    let delta =
                        self.replay_deltas
                            .get(&replay_delta_sequence)
                            .ok_or_else(|| {
                                JournalCodecError::new(
                                    "continuation_anchor replay group has no indexed replay delta",
                                )
                            })?;
                    let first_sequence = self
                        .completed_turns
                        .get(&delta.turn_id)
                        .copied()
                        .ok_or_else(|| {
                            JournalCodecError::new(
                                "continuation_anchor replay group has no completed Turn boundary",
                            )
                        })?;
                    if delta.epoch != anchor.epoch()
                        || delta.context_epoch != context_epoch
                        || delta.accepted_request_sequence != anchor.accepted_request_sequence()
                        || first_sequence > replay_delta_sequence
                        || replay_delta_sequence > sequence
                    {
                        return Err(JournalCodecError::new(
                            "continuation_anchor replay group coordinates are inconsistent",
                        ));
                    }
                    self.replay_groups.push(ReplayGroup {
                        first_sequence,
                        last_sequence: anchor.journal_boundary(),
                        replay_delta_sequence,
                        epoch: delta.epoch,
                        context_epoch,
                        items: delta.items.clone(),
                    });
                }
                self.latest_anchor = Some(sequence);
                self.request_after_checkpoint = false;
            },
            JournalRecord::ContextPolicyChanged(policy) => {
                if self.saw_legacy_context_record {
                    return Err(JournalCodecError::new(
                        "context policy cannot be introduced into a legacy context graph",
                    ));
                }
                let expected_revision = self
                    .current_policy
                    .as_ref()
                    .map_or(1, |current| current.policy_revision().saturating_add(1));
                if policy.policy_revision() != expected_revision {
                    return Err(JournalCodecError::new(
                        "context policy revisions must start at 1 and increase by exactly one",
                    ));
                }
                if self.open_epoch.is_none()
                    || !matches!(
                        self.open_strategy,
                        Some(ContinuationStrategy::ExactReplay {
                            executor: crate::ReplayExecutor::LocalClient,
                            ..
                        })
                    )
                {
                    return Err(JournalCodecError::new(
                        "context policy requires an open local-client exact-replay binding",
                    ));
                }
                self.context_epoch.get_or_insert(1);
                self.current_policy = Some(policy.clone());
            },
            JournalRecord::ContextCheckpoint(checkpoint) => {
                self.observe_checkpoint(sequence, checkpoint)?;
            },
            JournalRecord::SessionDescriptor(_)
            | JournalRecord::MessageReset(_)
            | JournalRecord::MessageSegment(_)
            | JournalRecord::MessageEnded(_) => {},
        }

        match record {
            JournalRecord::BackendExchangeObserved(exchange) => {
                self.reference_targets
                    .insert(sequence, ReferenceTarget::Exchange(exchange.into()));
            },
            JournalRecord::ContinuationAnchor(anchor) => {
                self.reference_targets.insert(
                    sequence,
                    ReferenceTarget::Anchor {
                        epoch: anchor.epoch(),
                        context_epoch: anchor.context_epoch(),
                        journal_boundary: anchor.journal_boundary(),
                    },
                );
            },
            JournalRecord::ContextCheckpoint(checkpoint) => {
                let replay_profile = match self.open_strategy {
                    Some(ContinuationStrategy::ExactReplay { replay_profile, .. }) => {
                        replay_profile
                    },
                    _ => unreachable!("a validated checkpoint has an exact-replay binding"),
                };
                let binding_identity = self
                    .open_binding_identity
                    .clone()
                    .expect("a validated checkpoint has an open binding identity");
                let has_provider_private = checkpoint
                    .retained_groups()
                    .iter()
                    .flat_map(|group| group.items())
                    .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }));
                self.reference_targets.insert(
                    sequence,
                    ReferenceTarget::Checkpoint {
                        epoch: checkpoint.epoch(),
                        context_epoch: checkpoint.successor_context_epoch(),
                        binding_identity,
                        replay_profile,
                        has_provider_private,
                    },
                );
            },
            _ => {},
        }
        if let (Some(epoch), Some(context_epoch)) = (self.open_epoch, self.context_epoch) {
            self.record_coordinates
                .insert(sequence, (epoch, context_epoch));
        }
        Ok(())
    }

    pub(super) const fn open_epoch(&self) -> Option<u64> {
        self.open_epoch
    }

    pub(super) const fn latest_anchor(&self) -> Option<JournalSequence> {
        self.latest_anchor
    }

    pub(super) const fn latest_checkpoint(&self) -> Option<JournalSequence> {
        if self.request_after_checkpoint || self.latest_anchor.is_some() {
            None
        } else {
            self.latest_checkpoint
        }
    }

    pub(super) const fn context_epoch(&self) -> Option<u64> {
        self.context_epoch
    }

    pub(super) const fn current_policy(&self) -> Option<&ContextPolicyChanged> {
        self.current_policy.as_ref()
    }

    pub(super) fn replay_groups(&self) -> Vec<Vec<ModelReplayItem>> {
        self.replay_groups
            .iter()
            .map(|group| group.items.clone())
            .collect()
    }

    pub(super) const fn model_replay(&self) -> &ModelReplay {
        &self.model_replay
    }

    pub(super) const fn replay_contract_rebind_required(&self) -> bool {
        self.replay_contract_rebind_required
    }

    fn observe_context_epoch(
        &mut self,
        record_context_epoch: Option<u64>,
        record_kind: &'static str,
    ) -> Result<(), JournalCodecError> {
        match (self.context_epoch, record_context_epoch) {
            (None, None) => {
                self.saw_legacy_context_record = true;
                Ok(())
            },
            (None, Some(_)) => Err(JournalCodecError::new(format!(
                "{record_kind} declares context_epoch before context policy revision 1"
            ))),
            (Some(_), None) => Err(JournalCodecError::new(format!(
                "{record_kind} omits context_epoch in a current context graph"
            ))),
            (Some(current), Some(record)) if current != record => Err(JournalCodecError::new(
                format!("{record_kind} does not name the current context_epoch"),
            )),
            (Some(_), Some(_)) => Ok(()),
        }
    }

    fn observe_checkpoint(
        &mut self,
        sequence: JournalSequence,
        checkpoint: &super::super::ContextCheckpoint,
    ) -> Result<(), JournalCodecError> {
        if self.saw_legacy_context_record {
            return Err(JournalCodecError::new(
                "context checkpoint cannot be introduced into a legacy context graph",
            ));
        }
        if self.open_epoch != Some(checkpoint.epoch())
            || !matches!(
                self.open_strategy,
                Some(ContinuationStrategy::ExactReplay {
                    executor: crate::ReplayExecutor::LocalClient,
                    ..
                })
            )
        {
            return Err(JournalCodecError::new(
                "context checkpoint requires its open local-client exact-replay binding",
            ));
        }
        let Some(policy) = &self.current_policy else {
            return Err(JournalCodecError::new(
                "context checkpoint requires a current context policy",
            ));
        };
        if !policy.enabled()
            || policy.policy_revision() != checkpoint.policy_revision()
            || policy.strategy() != checkpoint.strategy()
            || checkpoint.strategy() != ContextStrategy::PortableSummaryV1Alpha1
        {
            return Err(JournalCodecError::new(
                "context checkpoint does not match its enabled portable policy",
            ));
        }
        if self.context_epoch != Some(checkpoint.previous_context_epoch()) {
            return Err(JournalCodecError::new(
                "context checkpoint does not advance the current context_epoch",
            ));
        }
        let Some(ReferenceTarget::Anchor {
            epoch,
            context_epoch,
            journal_boundary,
        }) = self
            .reference_targets
            .get(&checkpoint.source_anchor_sequence())
            .cloned()
        else {
            return Err(JournalCodecError::new(
                "context checkpoint source does not identify an earlier continuation Anchor",
            ));
        };
        if epoch != checkpoint.epoch()
            || context_epoch != Some(checkpoint.previous_context_epoch())
            || journal_boundary > checkpoint.source_journal_boundary()
            || self
                .record_coordinates
                .get(&checkpoint.source_journal_boundary())
                != Some(&(checkpoint.epoch(), checkpoint.previous_context_epoch()))
            || checkpoint.source_journal_boundary() >= sequence
        {
            return Err(JournalCodecError::new(
                "context checkpoint source Anchor or boundary is inconsistent",
            ));
        }
        if self.model_replay.contract() != Some(checkpoint.replay_contract()) {
            return Err(JournalCodecError::new(
                "context checkpoint replay contract differs from its source binding",
            ));
        }
        self.validate_checkpoint_sources(checkpoint)?;
        let retained_items = checkpoint
            .retained_groups()
            .iter()
            .flat_map(|group| group.items());
        let has_provider_private = retained_items
            .clone()
            .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }));
        match self.open_strategy {
            Some(ContinuationStrategy::ExactReplay {
                replay_profile: ReplayProfile::SemanticOnly,
                ..
            }) if has_provider_private => {
                return Err(JournalCodecError::new(
                    "semantic-only context checkpoint cannot retain provider-private items",
                ));
            },
            Some(ContinuationStrategy::ExactReplay {
                replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                ..
            }) => {
                let items = retained_items.cloned().collect::<Vec<_>>();
                if items.iter().any(|item| {
                    matches!(
                        item,
                        ModelReplayItem::Message {
                            role: crate::ModelReplayRole::Assistant,
                            ..
                        }
                    )
                }) {
                    validate_provider_private_replay_sequence(
                        &items,
                        provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext)
                            .expect("the provider-private profile has an exact schema"),
                    )
                    .map_err(JournalCodecError::new)?;
                } else if items
                    .iter()
                    .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }))
                {
                    return Err(JournalCodecError::new(
                        "provider-private retained replay item has no assistant group",
                    ));
                }
            },
            _ => {},
        }
        self.model_replay = checkpoint.replay_root().map_err(|detail| {
            JournalCodecError::new(format!(
                "context checkpoint cannot establish its replay root: {detail}"
            ))
        })?;
        let root_items = self.model_replay.items().to_vec();
        self.context_epoch = Some(checkpoint.successor_context_epoch());
        self.latest_checkpoint = Some(sequence);
        self.request_after_checkpoint = false;
        self.latest_anchor = None;
        self.replay_deltas.clear();
        self.replay_groups.clear();
        self.replay_groups.push(ReplayGroup {
            first_sequence: sequence,
            last_sequence: sequence,
            replay_delta_sequence: sequence,
            epoch: checkpoint.epoch(),
            context_epoch: checkpoint.successor_context_epoch(),
            items: root_items,
        });
        Ok(())
    }

    fn validate_checkpoint_sources(
        &self,
        checkpoint: &super::super::ContextCheckpoint,
    ) -> Result<(), JournalCodecError> {
        let expected_coordinates = (checkpoint.epoch(), checkpoint.previous_context_epoch());
        let source_groups = self
            .replay_groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.last_sequence <= checkpoint.source_journal_boundary())
            .collect::<Vec<_>>();
        let mut retained = BTreeSet::new();
        let mut active_retained = Vec::new();
        for retained_group in checkpoint.retained_groups() {
            self.validate_source_range(
                retained_group.first_sequence(),
                retained_group.last_sequence(),
                expected_coordinates,
                "context retained group",
            )?;
            let source_group = source_groups.iter().find(|(_, source_group)| {
                source_group.first_sequence == retained_group.first_sequence()
                    && source_group.last_sequence == retained_group.last_sequence()
            });
            let Some((index, source_group)) = source_group else {
                if retained_group.first_sequence() > checkpoint.source_anchor_sequence()
                    && retained_group.last_sequence() <= checkpoint.source_journal_boundary()
                {
                    if !self.active_input_group_matches(retained_group) {
                        return Err(JournalCodecError::new(
                            "context retained active group is not the exact Journal-backed submitted input",
                        ));
                    }
                    active_retained.push(retained_group);
                    continue;
                }
                return Err(JournalCodecError::new(
                    "context retained range does not identify one whole completed replay group",
                ));
            };
            if source_group.epoch != checkpoint.epoch()
                || source_group.context_epoch != checkpoint.previous_context_epoch()
                || source_group.items != retained_group.items()
                || !retained.insert(*index)
            {
                return Err(JournalCodecError::new(
                    "context retained group is not the exact Journal-backed replay group",
                ));
            }
        }

        let mut summarized = BTreeSet::new();
        let mut declared_private_losses = Vec::new();
        for loss in checkpoint.losses() {
            match loss {
                ContextLoss::VisiblePrefixSummarized {
                    first_sequence,
                    last_sequence,
                } => {
                    if *last_sequence > checkpoint.source_anchor_sequence() {
                        return Err(JournalCodecError::new(
                            "visible summarized range crosses the mandatory active suffix",
                        ));
                    }
                    self.validate_source_range(
                        *first_sequence,
                        *last_sequence,
                        expected_coordinates,
                        "visible summarized range",
                    )?;
                    let covered = source_groups
                        .iter()
                        .filter(|(_, group)| {
                            group.first_sequence >= *first_sequence
                                && group.last_sequence <= *last_sequence
                        })
                        .collect::<Vec<_>>();
                    if covered.first().map(|(_, group)| group.first_sequence)
                        != Some(*first_sequence)
                        || covered.last().map(|(_, group)| group.last_sequence)
                            != Some(*last_sequence)
                    {
                        return Err(JournalCodecError::new(
                            "visible summarized range does not cover whole replay groups",
                        ));
                    }
                    for (index, _) in covered {
                        if retained.contains(index) || !summarized.insert(*index) {
                            return Err(JournalCodecError::new(
                                "context retained and summarized replay groups overlap",
                            ));
                        }
                    }
                },
                ContextLoss::ProviderPrivateDropped {
                    schema,
                    byte_count,
                    source_journal_sequence,
                } => {
                    if self.record_coordinates.get(source_journal_sequence)
                        != Some(&expected_coordinates)
                    {
                        return Err(JournalCodecError::new(
                            "provider-private context loss source is outside its binding or context epoch",
                        ));
                    }
                    declared_private_losses.push((
                        schema.clone(),
                        *byte_count,
                        *source_journal_sequence,
                    ));
                },
            }
        }

        if source_groups
            .iter()
            .any(|(index, _)| !retained.contains(index) && !summarized.contains(index))
        {
            return Err(JournalCodecError::new(
                "context checkpoint silently omits a source replay group",
            ));
        }

        for (source_sequence, coordinates) in self
            .record_coordinates
            .range(..=checkpoint.source_journal_boundary())
            .filter(|(sequence, _)| **sequence > checkpoint.source_anchor_sequence())
        {
            if *coordinates != expected_coordinates
                || !active_retained.iter().any(|group| {
                    group.first_sequence() <= *source_sequence
                        && *source_sequence <= group.last_sequence()
                })
            {
                return Err(JournalCodecError::new(
                    "context checkpoint does not retain the complete active semantic suffix",
                ));
            }
        }
        for (source_sequence, input) in self
            .submitted_inputs
            .range(..=checkpoint.source_journal_boundary())
            .filter(|(sequence, _)| **sequence > checkpoint.source_anchor_sequence())
        {
            let retained_input = active_retained.iter().find(|group| {
                group.first_sequence() <= *source_sequence
                    && *source_sequence <= group.last_sequence()
            });
            if !retained_input.is_some_and(|group| {
                group.items().iter().any(|item| {
                    matches!(
                        item,
                        ModelReplayItem::Message {
                            role: crate::ModelReplayRole::User,
                            content,
                            ..
                        } if content == input
                    )
                })
            }) {
                return Err(JournalCodecError::new(
                    "context checkpoint does not retain an active submitted input",
                ));
            }
        }

        let mut expected_private_losses = source_groups
            .iter()
            .filter(|(index, _)| summarized.contains(index))
            .flat_map(|(_, group)| {
                group.items.iter().filter_map(|item| match item {
                    ModelReplayItem::ProviderPrivateAssistant { envelope } => Some((
                        envelope.schema().to_owned(),
                        u64::try_from(envelope.payload().len())
                            .expect("provider-private replay byte bounds fit u64"),
                        group.replay_delta_sequence,
                    )),
                    _ => None,
                })
            })
            .collect::<Vec<_>>();
        expected_private_losses.sort();
        declared_private_losses.sort();
        if expected_private_losses != declared_private_losses {
            return Err(JournalCodecError::new(
                "provider-private loss disclosure does not exactly cover summarized private replay",
            ));
        }

        let mut artifact_identities = BTreeSet::new();
        for receipt in checkpoint.artifact_receipts() {
            let Some((group_index, group)) = source_groups.iter().find(|(_, group)| {
                group.replay_delta_sequence == receipt.source_journal_sequence()
            }) else {
                return Err(JournalCodecError::new(
                    "context artifact receipt source is not a completed replay group",
                ));
            };
            if !summarized.contains(group_index)
                || !group
                    .items
                    .iter()
                    .any(|item| artifact_matches(item, receipt))
                || !artifact_identities.insert((
                    receipt.source_journal_sequence(),
                    receipt.content_hash().to_owned(),
                    receipt.byte_count(),
                    receipt.media_kind().to_owned(),
                ))
            {
                return Err(JournalCodecError::new(
                    "context artifact receipt is duplicated or not bound to visible summarized replay",
                ));
            }
        }
        Ok(())
    }

    fn active_input_group_matches(&self, group: &super::super::ContextRetainedGroup) -> bool {
        let Some(input) = self.submitted_inputs.get(&group.first_sequence()) else {
            return false;
        };
        let Some(ModelReplayItem::Message {
            role: crate::ModelReplayRole::User,
            content,
            refusal: None,
        }) = group.items().first()
        else {
            return false;
        };
        if content != input
            || self
                .submitted_inputs
                .range(group.first_sequence()..=group.last_sequence())
                .count()
                != 1
        {
            return false;
        }
        if group.first_sequence() == group.last_sequence() {
            return group.items().len() == 1;
        }
        self.completed_activity_boundaries
            .contains(&group.last_sequence())
            && group
                .items()
                .iter()
                .any(|item| matches!(item, ModelReplayItem::FunctionCall { .. }))
            && group
                .items()
                .iter()
                .any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { .. }))
    }

    fn validate_source_range(
        &self,
        first: JournalSequence,
        last: JournalSequence,
        expected: (u64, u64),
        label: &str,
    ) -> Result<(), JournalCodecError> {
        if first > last
            || self.record_coordinates.get(&first) != Some(&expected)
            || self.record_coordinates.get(&last) != Some(&expected)
            || self
                .record_coordinates
                .range(first..=last)
                .any(|(_, coordinates)| *coordinates != expected)
        {
            return Err(JournalCodecError::new(format!(
                "{label} crosses its binding or context epoch"
            )));
        }
        Ok(())
    }

    fn observe_command(
        &mut self,
        sequence: JournalSequence,
        command: &crate::journal::CommittedCommand,
    ) {
        let Some(submission_id) = command.submission_id() else {
            return;
        };
        let turn_id = match command.command() {
            AgentCommand::StartTurn { turn, input } => {
                self.active_turn_starts.insert(turn.turn_id(), sequence);
                self.submitted_inputs
                    .insert(sequence, input.as_str().to_owned());
                turn.turn_id()
            },
            AgentCommand::SteerTurn { turn, input } => {
                self.submitted_inputs
                    .insert(sequence, input.as_str().to_owned());
                turn.turn_id()
            },
            AgentCommand::CreateSession { .. }
            | AgentCommand::InterruptTurn { .. }
            | AgentCommand::CompactContext { .. }
            | AgentCommand::RespondToActivity { .. } => return,
        };
        self.submission_commands
            .insert(OperationId::from(submission_id), turn_id);
    }

    fn observe_event(&mut self, sequence: JournalSequence, event: &AgentEvent) {
        match event {
            AgentEvent::SessionCreated { session_id } => {
                self.session_id = Some(*session_id);
                self.session_created = true;
            },
            AgentEvent::TurnFinished { turn, outcome } => {
                self.active_turn_starts.remove(&turn.turn_id());
                if matches!(outcome, TurnOutcome::Completed) {
                    self.completed_turns.insert(turn.turn_id(), sequence);
                } else {
                    self.completed_turns.remove(&turn.turn_id());
                }
            },
            AgentEvent::TurnStarted { turn } => {
                self.completed_turns.remove(&turn.turn_id());
            },
            AgentEvent::ActivityFinished { .. } => {
                self.completed_activity_boundaries.insert(sequence);
            },
            AgentEvent::ActivityStarted { .. } | AgentEvent::ActivityUpdated { .. } => {},
        }
    }

    fn observe_exchange(
        &mut self,
        sequence: JournalSequence,
        exchange: &super::super::BackendExchangeObserved,
    ) -> Result<(), JournalCodecError> {
        if self.open_epoch != Some(exchange.epoch()) {
            return Err(JournalCodecError::new(
                "backend_exchange_observed requires its open binding epoch",
            ));
        }
        let correlation = exchange.correlation_sequence();
        match exchange.kind() {
            ExchangeKind::Request | ExchangeKind::ServerRequest | ExchangeKind::Notification => {
                if correlation.is_some() {
                    return Err(JournalCodecError::new(
                        "root request, server request, and notification exchanges cannot have a correlation edge",
                    ));
                }
                if !self.operation_roots.insert(exchange.operation_id()) {
                    return Err(JournalCodecError::new(
                        "one operation_id cannot begin a second root exchange",
                    ));
                }
            },
            ExchangeKind::Response => {
                let target = self.exchange_target(exchange)?;
                if !matches!(
                    target.kind,
                    ExchangeKind::Request | ExchangeKind::ServerRequest
                ) || target.direction == exchange.direction()
                {
                    return Err(JournalCodecError::new(
                        "response must reference an opposite-direction request or server request",
                    ));
                }
            },
            ExchangeKind::Retry => {
                let target = self.exchange_target(exchange)?;
                if !matches!(
                    target.kind,
                    ExchangeKind::Request | ExchangeKind::ServerRequest | ExchangeKind::Retry
                ) || target.direction != exchange.direction()
                {
                    return Err(JournalCodecError::new(
                        "retry must reference a same-direction request, server request, or retry",
                    ));
                }
            },
            ExchangeKind::TerminalOutcome => {
                let target = self.exchange_target(exchange)?;
                if !matches!(
                    target.kind,
                    ExchangeKind::Request
                        | ExchangeKind::ServerRequest
                        | ExchangeKind::Retry
                        | ExchangeKind::Response
                ) {
                    return Err(JournalCodecError::new(
                        "terminal outcome has an invalid correlation target",
                    ));
                }
            },
        }
        if exchange.kind() == ExchangeKind::Request
            && exchange.direction() == ExchangeDirection::YoToBackend
        {
            self.latest_request_exchange
                .insert((exchange.epoch(), exchange.operation_id()), sequence);
        }
        Ok(())
    }

    fn exchange_target(
        &self,
        exchange: &super::super::BackendExchangeObserved,
    ) -> Result<IndexedExchange, JournalCodecError> {
        let sequence = exchange.correlation_sequence().ok_or_else(|| {
            JournalCodecError::new("correlated exchange is missing correlation_sequence")
        })?;
        let Some(ReferenceTarget::Exchange(target)) =
            self.reference_targets.get(&sequence).cloned()
        else {
            return Err(JournalCodecError::new(
                "exchange correlation_sequence does not identify an earlier exchange",
            ));
        };
        if target.epoch != exchange.epoch() || target.operation_id != exchange.operation_id() {
            return Err(JournalCodecError::new(
                "correlated exchanges must share epoch and operation_id",
            ));
        }
        Ok(target)
    }

    fn observe_binding_open(
        &mut self,
        sequence: JournalSequence,
        binding: &super::super::BackendBindingOpened,
    ) -> Result<(), JournalCodecError> {
        if self.open_epoch.is_some() {
            return Err(JournalCodecError::new(
                "at most one backend binding epoch may be open",
            ));
        }
        let is_replacement = self.last_epoch.is_some();
        let seed_items = self.model_replay.items().to_vec();
        match self.last_epoch {
            None => {
                if binding.epoch() != 1
                    || !self.session_created
                    || binding.transition().mode() != TransitionMode::Initial
                    || binding.transition().cache() != CacheState::NotApplicable
                    || binding.transition().source_anchor_sequence().is_some()
                    || binding.transition().source_checkpoint_sequence().is_some()
                {
                    return Err(JournalCodecError::new(
                        "first backend binding must open epoch 1 after SessionCreated with an initial transition",
                    ));
                }
            },
            Some(previous) => {
                if binding.epoch() != previous.saturating_add(1)
                    || self.last_close_reason != Some(BindingCloseReason::Replaced)
                    || binding.transition().mode() == TransitionMode::Initial
                {
                    return Err(JournalCodecError::new(
                        "replacement binding must follow a replaced epoch with the next number",
                    ));
                }
                let anchor_source = binding.transition().source_anchor_sequence();
                let checkpoint_source = binding.transition().source_checkpoint_sequence();
                match (anchor_source, checkpoint_source) {
                    (Some(source), None) => {
                        let Some(ReferenceTarget::Anchor {
                            epoch,
                            context_epoch,
                            ..
                        }) = self.reference_targets.get(&source).cloned()
                        else {
                            return Err(JournalCodecError::new(
                                "replacement binding source does not identify a continuation Anchor",
                            ));
                        };
                        let inherited_local_replay_anchor = binding.continuation_strategy()
                            == (ContinuationStrategy::ExactReplay {
                                executor: crate::ReplayExecutor::LocalClient,
                                replay_profile: crate::ReplayProfile::SemanticOnly,
                            })
                            && binding.transition().mode() == TransitionMode::ExactReplay;
                        let backend_native_model_rebind = binding.continuation_strategy()
                            == ContinuationStrategy::BackendManagedState
                            && binding.transition().mode()
                                == TransitionMode::BackendNativeModelRebind;
                        if self.replacement_source
                            != Some(ReplacementSource::Anchor {
                                sequence: source,
                                epoch: previous,
                            })
                            || (epoch != previous
                                && !inherited_local_replay_anchor
                                && !backend_native_model_rebind)
                            || context_epoch != self.context_epoch
                        {
                            return Err(JournalCodecError::new(
                                "replacement binding must use the valid resume Anchor carried by the immediately preceding epoch",
                            ));
                        }
                        if inherited_local_replay_anchor {
                            self.latest_anchor = Some(source);
                        }
                    },
                    (None, Some(source)) => {
                        let Some(ReferenceTarget::Checkpoint {
                            epoch: _,
                            context_epoch,
                            binding_identity: source_binding_identity,
                            replay_profile: source_replay_profile,
                            has_provider_private,
                        }) = self.reference_targets.get(&source).cloned()
                        else {
                            return Err(JournalCodecError::new(
                                "replacement binding source does not identify a context checkpoint",
                            ));
                        };
                        let target_replay_profile = match binding.continuation_strategy() {
                            ContinuationStrategy::ExactReplay { replay_profile, .. } => {
                                Some(replay_profile)
                            },
                            ContinuationStrategy::BackendManagedState => None,
                        };
                        let private_seed_is_compatible = !has_provider_private
                            || (target_replay_profile == Some(source_replay_profile)
                                && binding.binding_identity() == &source_binding_identity);
                        if binding.transition().mode() != TransitionMode::ExactReplay
                            || self.context_epoch != Some(context_epoch)
                            || !private_seed_is_compatible
                            || self.replacement_source
                                != Some(ReplacementSource::Checkpoint {
                                    sequence: source,
                                    epoch: previous,
                                })
                        {
                            return Err(JournalCodecError::new(
                                "replacement binding must use the newest checkpoint-only reconstruction and preserve every retained private item",
                            ));
                        }
                    },
                    (None, None)
                        if binding.transition().mode()
                            == TransitionMode::BackendNativeModelRebind
                            && binding.continuation_strategy()
                                == ContinuationStrategy::BackendManagedState
                            && self.replacement_without_source_allowed => {},
                    _ => {
                        return Err(JournalCodecError::new(
                            "replacement binding requires an eligible Anchor, checkpoint, or source-free native model rebind",
                        ));
                    },
                }
            },
        }
        self.open_epoch = Some(binding.epoch());
        self.open_strategy = Some(binding.continuation_strategy());
        self.open_binding_identity = Some(binding.binding_identity().clone());
        if binding.continuation_strategy() == ContinuationStrategy::BackendManagedState
            || binding.transition().mode() != TransitionMode::ExactReplay
        {
            self.model_replay = ModelReplay::default();
            self.replay_contract_rebind_required = false;
        } else {
            self.replay_contract_rebind_required = is_replacement;
        }
        if is_replacement {
            self.replay_deltas.clear();
            self.replay_groups.clear();
            if matches!(
                binding.continuation_strategy(),
                ContinuationStrategy::ExactReplay { .. }
            ) && !seed_items.is_empty()
                && let Some(context_epoch) = self.context_epoch
            {
                self.replay_groups.push(ReplayGroup {
                    first_sequence: sequence,
                    last_sequence: sequence,
                    replay_delta_sequence: sequence,
                    epoch: binding.epoch(),
                    context_epoch,
                    items: seed_items,
                });
            }
        }
        self.last_epoch = Some(binding.epoch());
        self.last_close_reason = None;
        self.replacement_source = None;
        self.replacement_without_source_allowed = false;
        self.open_epoch_has_accepted_request = false;
        Ok(())
    }
}
