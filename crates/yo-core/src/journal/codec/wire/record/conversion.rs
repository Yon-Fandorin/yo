use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    super::{
        JournalCodecError,
        command::WireCommand,
        correlation,
        correlation::{
            WireBindingCloseReason, WireBindingTransition, WireContinuationStrategy,
            WireDetailAvailability, WireExchangeDirection, WireExchangeKind, WireResumableStatus,
            WireVersionedIdentity,
        },
        descriptor::WireSessionDescriptor,
        event::WireEvent,
        message::{WireMessageEnded, WireMessageReset, WireMessageSegment},
    },
    WireContextArtifactReceipt, WireContextLoss, WireContextRetainedGroup, WireForkRecord,
    WireModelReplayContract, WireModelReplayDelta, decode_context_loss, decode_model_replay,
    decode_model_replay_contract, deserialize_context_losses, encode_context_loss,
    encode_model_replay, encode_model_replay_contract, non_null_accounting_field,
    required_journal_sequence, with_context_epoch,
};
use crate::{
    AgentEvent, JournalSequence, SessionDescriptor, SessionId,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, CONTEXT_ARTIFACT_PROFILE,
        CONTEXT_CHECKPOINT_PROFILE, CONTEXT_POLICY_PROFILE, ContextArtifactReceipt,
        ContextCheckpoint, ContextPolicyChanged, ContextStrategy, ContextSummaryUsage,
        ContinuationAnchor, ForkHistoryCoordinate, ForkSeed, ForkSource,
        IMAGE_CONTEXT_CHECKPOINT_PROFILE, InitialForkSeed, JournalRecord, MessageEnded,
        MessageReset, MessageSegment, MessageTerminal, ModelReplayDeltaRecord,
        SequencedJournalRecord,
    },
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(in super::super) enum WireRecord {
    InitialForkSeed(Box<WireForkRecord>),
    SessionDescriptor {
        descriptor: WireSessionDescriptor,
    },
    CommandCommitted {
        journal_sequence: u64,
        command: WireCommand,
    },
    EventCommitted {
        journal_sequence: u64,
        event: WireEvent,
    },
    BackendExchangeObserved {
        journal_sequence: u64,
        epoch: u64,
        operation_id: String,
        exchange_kind: WireExchangeKind,
        direction: WireExchangeDirection,
        payload_schema: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        correlation_sequence: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exchange_identity: Option<WireVersionedIdentity>,
        detail_availability: WireDetailAvailability,
    },
    BackendBindingOpened {
        journal_sequence: u64,
        epoch: u64,
        backend_kind: String,
        backend_version: String,
        binding_identity: WireVersionedIdentity,
        model_identity: WireVersionedIdentity,
        session_locator: WireVersionedIdentity,
        transition: WireBindingTransition,
        continuation_strategy: WireContinuationStrategy,
    },
    BackendBindingClosed {
        journal_sequence: u64,
        epoch: u64,
        reason: WireBindingCloseReason,
    },
    BackendRequestAccepted {
        journal_sequence: u64,
        epoch: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_epoch: Option<u64>,
        turn_id: u64,
        operation_id: String,
        exchange_sequence: u64,
        request_identity: WireVersionedIdentity,
    },
    ModelReplayDelta {
        journal_sequence: u64,
        epoch: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_epoch: Option<u64>,
        turn_id: u64,
        accepted_request_sequence: u64,
        #[serde(flatten)]
        replay: WireModelReplayDelta,
    },
    BackendResumableOutcome {
        journal_sequence: u64,
        epoch: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_epoch: Option<u64>,
        turn_id: u64,
        accepted_request_sequence: u64,
        status: WireResumableStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome_identity: Option<WireVersionedIdentity>,
        #[serde(skip_serializing_if = "Option::is_none")]
        replay_delta_sequence: Option<u64>,
    },
    ContinuationAnchor {
        journal_sequence: u64,
        epoch: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_epoch: Option<u64>,
        accepted_request_sequence: u64,
        resumable_outcome_sequence: u64,
        journal_boundary: u64,
    },
    ContextPolicyChanged {
        journal_sequence: u64,
        profile: String,
        policy_revision: u64,
        enabled: bool,
        strategy: String,
        warning_percent: u8,
        trigger_percent: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        retained_raw_percent: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        retained_raw_max_tokens: Option<u64>,
    },
    ContextCheckpoint {
        journal_sequence: u64,
        profile: String,
        epoch: u64,
        previous_context_epoch: u64,
        successor_context_epoch: u64,
        source_anchor_sequence: u64,
        source_journal_boundary: u64,
        policy_revision: u64,
        strategy: String,
        input_token_limit: u64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "non_null_accounting_field"
        )]
        input_tokens_before: Option<u64>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "non_null_accounting_field"
        )]
        input_tokens_after: Option<u64>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "non_null_accounting_field"
        )]
        accounting_before: Option<crate::ContextAccounting>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "non_null_accounting_field"
        )]
        accounting_after: Option<crate::ContextAccounting>,
        replay_contract: WireModelReplayContract,
        portable_body: String,
        retained_groups: Vec<WireContextRetainedGroup>,
        #[serde(skip_serializing_if = "Option::is_none")]
        first_retained_sequence: Option<u64>,
        artifact_receipts: Vec<WireContextArtifactReceipt>,
        #[serde(deserialize_with = "deserialize_context_losses")]
        losses: Vec<WireContextLoss>,
        summary_usage: Value,
    },
    MessageReset {
        reset: WireMessageReset,
    },
    MessageSegment {
        segment: WireMessageSegment,
    },
    MessageEnded {
        #[serde(skip_serializing_if = "Option::is_none")]
        final_segment: Option<WireMessageSegment>,
        ended: WireMessageEnded,
    },
}

impl TryFrom<&SequencedJournalRecord> for WireRecord {
    type Error = JournalCodecError;

    fn try_from(entry: &SequencedJournalRecord) -> Result<Self, Self::Error> {
        let record = entry.record();
        Ok(match record {
            JournalRecord::InitialForkSeed(seed) => Self::InitialForkSeed(Box::new(
                WireForkRecord::encode(required_journal_sequence(entry)?, seed)?,
            )),
            JournalRecord::SessionDescriptor(descriptor) => Self::SessionDescriptor {
                descriptor: WireSessionDescriptor::from(descriptor),
            },
            JournalRecord::CommandCommitted(command) => Self::CommandCommitted {
                journal_sequence: required_journal_sequence(entry)?.get(),
                command: WireCommand::try_from(command)?,
            },
            JournalRecord::EventCommitted(event) => Self::EventCommitted {
                journal_sequence: required_journal_sequence(entry)?.get(),
                event: WireEvent::from(event),
            },
            JournalRecord::BackendExchangeObserved(exchange) => Self::BackendExchangeObserved {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: exchange.epoch(),
                operation_id: exchange.operation_id().as_uuid().to_string(),
                exchange_kind: exchange.kind().into(),
                direction: exchange.direction().into(),
                payload_schema: {
                    correlation::validate_ascii(exchange.payload_schema(), "payload_schema")?;
                    exchange.payload_schema().to_owned()
                },
                correlation_sequence: exchange.correlation_sequence().map(JournalSequence::get),
                exchange_identity: exchange
                    .exchange_identity()
                    .map(correlation::encode_identity)
                    .transpose()?,
                detail_availability: exchange.detail_availability().into(),
            },
            JournalRecord::BackendBindingOpened(binding) => Self::BackendBindingOpened {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: binding.epoch(),
                backend_kind: {
                    correlation::validate_ascii(binding.backend_kind(), "backend_kind")?;
                    binding.backend_kind().to_owned()
                },
                backend_version: {
                    correlation::validate_value(binding.backend_version(), "backend_version")?;
                    binding.backend_version().to_owned()
                },
                binding_identity: correlation::encode_identity(binding.binding_identity())?,
                model_identity: correlation::encode_identity(binding.model_identity())?,
                session_locator: correlation::encode_identity(binding.session_locator())?,
                transition: WireBindingTransition::encode(binding.transition())?,
                continuation_strategy: binding.continuation_strategy().into(),
            },
            JournalRecord::BackendBindingClosed(binding) => Self::BackendBindingClosed {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: binding.epoch(),
                reason: binding.reason().into(),
            },
            JournalRecord::BackendRequestAccepted(request) => Self::BackendRequestAccepted {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: request.epoch(),
                context_epoch: request.context_epoch(),
                turn_id: request.turn_id().get().get(),
                operation_id: request.operation_id().as_uuid().to_string(),
                exchange_sequence: request.exchange_sequence().get(),
                request_identity: correlation::encode_identity(request.request_identity())?,
            },
            JournalRecord::ModelReplayDelta(replay) => Self::ModelReplayDelta {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: replay.epoch(),
                context_epoch: replay.context_epoch(),
                turn_id: replay.turn_id().get().get(),
                accepted_request_sequence: replay.accepted_request_sequence().get(),
                replay: encode_model_replay(replay.delta(), replay.epoch()),
            },
            JournalRecord::BackendResumableOutcome(outcome) => Self::BackendResumableOutcome {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: outcome.epoch(),
                context_epoch: outcome.context_epoch(),
                turn_id: outcome.turn_id().get().get(),
                accepted_request_sequence: outcome.accepted_request_sequence().get(),
                status: WireResumableStatus::Completed,
                outcome_identity: outcome
                    .outcome_identity()
                    .map(correlation::encode_identity)
                    .transpose()?,
                replay_delta_sequence: outcome.replay_delta_sequence().map(JournalSequence::get),
            },
            JournalRecord::ContinuationAnchor(anchor) => Self::ContinuationAnchor {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: anchor.epoch(),
                context_epoch: anchor.context_epoch(),
                accepted_request_sequence: anchor.accepted_request_sequence().get(),
                resumable_outcome_sequence: anchor.resumable_outcome_sequence().get(),
                journal_boundary: anchor.journal_boundary().get(),
            },
            JournalRecord::ContextPolicyChanged(policy) => Self::ContextPolicyChanged {
                journal_sequence: required_journal_sequence(entry)?.get(),
                profile: CONTEXT_POLICY_PROFILE.to_owned(),
                policy_revision: policy.policy_revision(),
                enabled: policy.enabled(),
                strategy: policy.strategy().as_str().to_owned(),
                warning_percent: policy.warning_percent(),
                trigger_percent: policy.trigger_percent(),
                retained_raw_percent: policy.retained_raw_percent(),
                retained_raw_max_tokens: policy.retained_raw_max_tokens(),
            },
            JournalRecord::ContextCheckpoint(checkpoint) => {
                checkpoint
                    .validate_profile()
                    .map_err(JournalCodecError::new)?;
                Self::ContextCheckpoint {
                    journal_sequence: required_journal_sequence(entry)?.get(),
                    profile: checkpoint.profile().to_owned(),
                    epoch: checkpoint.epoch(),
                    previous_context_epoch: checkpoint.previous_context_epoch(),
                    successor_context_epoch: checkpoint.successor_context_epoch(),
                    source_anchor_sequence: checkpoint.source_anchor_sequence().get(),
                    source_journal_boundary: checkpoint.source_journal_boundary().get(),
                    policy_revision: checkpoint.policy_revision(),
                    strategy: checkpoint.strategy().as_str().to_owned(),
                    input_token_limit: checkpoint.input_token_limit(),
                    input_tokens_before: checkpoint
                        .accounting()
                        .is_none()
                        .then_some(checkpoint.input_tokens_before()),
                    input_tokens_after: checkpoint
                        .accounting()
                        .is_none()
                        .then_some(checkpoint.input_tokens_after()),
                    accounting_before: checkpoint.accounting().map(|(before, _)| before.clone()),
                    accounting_after: checkpoint.accounting().map(|(_, after)| after.clone()),
                    replay_contract: encode_model_replay_contract(checkpoint.replay_contract()),
                    portable_body: checkpoint.portable_body().to_owned(),
                    retained_groups: checkpoint
                        .retained_groups()
                        .iter()
                        .map(|group| WireContextRetainedGroup::encode(group, checkpoint.epoch()))
                        .collect(),
                    first_retained_sequence: checkpoint
                        .first_retained_sequence()
                        .map(JournalSequence::get),
                    artifact_receipts: checkpoint
                        .artifact_receipts()
                        .iter()
                        .map(|receipt| WireContextArtifactReceipt {
                            profile: CONTEXT_ARTIFACT_PROFILE.to_owned(),
                            content_hash: receipt.content_hash().to_owned(),
                            byte_count: receipt.byte_count(),
                            media_kind: receipt.media_kind().to_owned(),
                            source_context_epoch: receipt.source_context_epoch(),
                            source_journal_sequence: receipt.source_journal_sequence().get(),
                        })
                        .collect(),
                    losses: checkpoint
                        .losses()
                        .iter()
                        .map(encode_context_loss)
                        .collect(),
                    summary_usage: checkpoint.summary_usage().value().clone(),
                }
            },
            JournalRecord::MessageReset(reset) => Self::MessageReset {
                reset: WireMessageReset::from(reset),
            },
            JournalRecord::MessageSegment(segment) => Self::MessageSegment {
                segment: WireMessageSegment::from(segment),
            },
            JournalRecord::MessageEnded(terminal) => Self::MessageEnded {
                final_segment: terminal.final_segment().map(WireMessageSegment::from),
                ended: WireMessageEnded::from(terminal.ended()),
            },
        })
    }
}

impl TryFrom<WireRecord> for (Option<JournalSequence>, JournalRecord) {
    type Error = JournalCodecError;

    fn try_from(record: WireRecord) -> Result<Self, Self::Error> {
        match record {
            WireRecord::InitialForkSeed(_) => Err(JournalCodecError::new(
                "initial fork seed requires its preceding child descriptor",
            )),
            WireRecord::SessionDescriptor { descriptor } => Ok((
                None,
                JournalRecord::SessionDescriptor(SessionDescriptor::try_from(descriptor)?),
            )),
            WireRecord::CommandCommitted {
                journal_sequence,
                command,
            } => Ok((
                Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                JournalRecord::CommandCommitted(command.try_into()?),
            )),
            WireRecord::EventCommitted {
                journal_sequence,
                event,
            } => Ok((
                Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                JournalRecord::EventCommitted(AgentEvent::try_from(event)?),
            )),
            WireRecord::BackendExchangeObserved {
                journal_sequence,
                epoch,
                operation_id,
                exchange_kind,
                direction,
                payload_schema,
                correlation_sequence,
                exchange_identity,
                detail_availability,
            } => {
                correlation::positive(epoch, "epoch")?;
                correlation::validate_ascii(&payload_schema, "payload_schema")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                        epoch,
                        correlation::operation_id(operation_id)?,
                        exchange_kind.into(),
                        direction.into(),
                        payload_schema,
                        correlation_sequence
                            .map(|value| correlation::sequence(value, "correlation_sequence"))
                            .transpose()?,
                        exchange_identity
                            .map(correlation::decode_identity)
                            .transpose()?,
                        detail_availability.into(),
                    )),
                ))
            },
            WireRecord::BackendBindingOpened {
                journal_sequence,
                epoch,
                backend_kind,
                backend_version,
                binding_identity,
                model_identity,
                session_locator,
                transition,
                continuation_strategy,
            } => {
                correlation::positive(epoch, "epoch")?;
                correlation::validate_ascii(&backend_kind, "backend_kind")?;
                correlation::validate_value(&backend_version, "backend_version")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                        epoch,
                        backend_kind,
                        backend_version,
                        correlation::decode_identity(binding_identity)?,
                        correlation::decode_identity(model_identity)?,
                        correlation::decode_identity(session_locator)?,
                        transition.decode()?,
                        continuation_strategy.try_into()?,
                    )),
                ))
            },
            WireRecord::BackendBindingClosed {
                journal_sequence,
                epoch,
                reason,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                        epoch,
                        reason.into(),
                    )),
                ))
            },
            WireRecord::BackendRequestAccepted {
                journal_sequence,
                epoch,
                context_epoch,
                turn_id,
                operation_id,
                exchange_sequence,
                request_identity,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendRequestAccepted(with_context_epoch(
                        BackendRequestAccepted::new(
                            epoch,
                            correlation::turn_id(turn_id)?,
                            correlation::operation_id(operation_id)?,
                            correlation::sequence(exchange_sequence, "exchange_sequence")?,
                            correlation::decode_identity(request_identity)?,
                        ),
                        context_epoch,
                        BackendRequestAccepted::with_context_epoch,
                    )?),
                ))
            },
            WireRecord::ModelReplayDelta {
                journal_sequence,
                epoch,
                context_epoch,
                turn_id,
                accepted_request_sequence,
                replay,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::ModelReplayDelta(with_context_epoch(
                        ModelReplayDeltaRecord::new(
                            epoch,
                            correlation::turn_id(turn_id)?,
                            correlation::sequence(
                                accepted_request_sequence,
                                "accepted_request_sequence",
                            )?,
                            decode_model_replay(replay, epoch)?,
                        ),
                        context_epoch,
                        ModelReplayDeltaRecord::with_context_epoch,
                    )?),
                ))
            },
            WireRecord::BackendResumableOutcome {
                journal_sequence,
                epoch,
                context_epoch,
                turn_id,
                accepted_request_sequence,
                status: WireResumableStatus::Completed,
                outcome_identity,
                replay_delta_sequence,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendResumableOutcome(with_context_epoch(
                        BackendResumableOutcome::new(
                            epoch,
                            correlation::turn_id(turn_id)?,
                            correlation::sequence(
                                accepted_request_sequence,
                                "accepted_request_sequence",
                            )?,
                            outcome_identity
                                .map(correlation::decode_identity)
                                .transpose()?,
                            replay_delta_sequence
                                .map(|value| correlation::sequence(value, "replay_delta_sequence"))
                                .transpose()?,
                        ),
                        context_epoch,
                        BackendResumableOutcome::with_context_epoch,
                    )?),
                ))
            },
            WireRecord::ContinuationAnchor {
                journal_sequence,
                epoch,
                context_epoch,
                accepted_request_sequence,
                resumable_outcome_sequence,
                journal_boundary,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::ContinuationAnchor(with_context_epoch(
                        ContinuationAnchor::new(
                            epoch,
                            correlation::sequence(
                                accepted_request_sequence,
                                "accepted_request_sequence",
                            )?,
                            correlation::sequence(
                                resumable_outcome_sequence,
                                "resumable_outcome_sequence",
                            )?,
                            correlation::sequence(journal_boundary, "journal_boundary")?,
                        ),
                        context_epoch,
                        ContinuationAnchor::with_context_epoch,
                    )?),
                ))
            },
            WireRecord::ContextPolicyChanged {
                journal_sequence,
                profile,
                policy_revision,
                enabled,
                strategy,
                warning_percent,
                trigger_percent,
                retained_raw_percent,
                retained_raw_max_tokens,
            } => {
                if profile != CONTEXT_POLICY_PROFILE {
                    return Err(JournalCodecError::new("unsupported context policy profile"));
                }
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::ContextPolicyChanged(
                        ContextPolicyChanged::try_new(
                            policy_revision,
                            enabled,
                            ContextStrategy::parse(&strategy).map_err(JournalCodecError::new)?,
                            warning_percent,
                            trigger_percent,
                            retained_raw_percent,
                            retained_raw_max_tokens,
                        )
                        .map_err(JournalCodecError::new)?,
                    ),
                ))
            },
            WireRecord::ContextCheckpoint {
                journal_sequence,
                profile,
                epoch,
                previous_context_epoch,
                successor_context_epoch,
                source_anchor_sequence,
                source_journal_boundary,
                policy_revision,
                strategy,
                input_token_limit,
                input_tokens_before,
                input_tokens_after,
                accounting_before,
                accounting_after,
                replay_contract,
                portable_body,
                retained_groups,
                first_retained_sequence,
                artifact_receipts,
                losses,
                summary_usage,
            } => {
                if profile != CONTEXT_CHECKPOINT_PROFILE
                    && profile != IMAGE_CONTEXT_CHECKPOINT_PROFILE
                {
                    return Err(JournalCodecError::new(
                        "unsupported context checkpoint profile",
                    ));
                }
                let (before, after, accounting) = match (
                    profile.as_str(),
                    input_tokens_before,
                    input_tokens_after,
                    accounting_before,
                    accounting_after,
                ) {
                    (CONTEXT_CHECKPOINT_PROFILE, Some(before), Some(after), None, None) => {
                        (before, after, None)
                    },
                    (IMAGE_CONTEXT_CHECKPOINT_PROFILE, None, None, Some(before), Some(after)) => (
                        before.planning_tokens(),
                        after.planning_tokens(),
                        Some((before, after)),
                    ),
                    _ => {
                        return Err(JournalCodecError::new(
                            "checkpoint profile has missing or mixed accounting fields",
                        ));
                    },
                };
                correlation::positive(epoch, "epoch")?;
                let retained_groups = retained_groups
                    .into_iter()
                    .map(|group| group.decode(epoch))
                    .collect::<Result<Vec<_>, JournalCodecError>>()?;
                let artifact_receipts = artifact_receipts
                    .into_iter()
                    .map(|receipt| {
                        if receipt.profile != CONTEXT_ARTIFACT_PROFILE {
                            return Err(JournalCodecError::new(
                                "unsupported context artifact receipt profile",
                            ));
                        }
                        ContextArtifactReceipt::try_new(
                            receipt.content_hash,
                            receipt.byte_count,
                            receipt.media_kind,
                            receipt.source_context_epoch,
                            correlation::sequence(
                                receipt.source_journal_sequence,
                                "source_journal_sequence",
                            )?,
                        )
                        .map_err(JournalCodecError::new)
                    })
                    .collect::<Result<Vec<_>, JournalCodecError>>()?;
                let losses = losses
                    .into_iter()
                    .map(decode_context_loss)
                    .collect::<Result<Vec<_>, JournalCodecError>>()?;
                let mut checkpoint = ContextCheckpoint::try_new(
                    epoch,
                    previous_context_epoch,
                    successor_context_epoch,
                    correlation::sequence(source_anchor_sequence, "source_anchor_sequence")?,
                    correlation::sequence(source_journal_boundary, "source_journal_boundary")?,
                    policy_revision,
                    ContextStrategy::parse(&strategy).map_err(JournalCodecError::new)?,
                    input_token_limit,
                    before,
                    after,
                    decode_model_replay_contract(replay_contract),
                    portable_body,
                    retained_groups,
                    first_retained_sequence
                        .map(|value| correlation::sequence(value, "first_retained_sequence"))
                        .transpose()?,
                    artifact_receipts,
                    losses,
                    ContextSummaryUsage::try_new(summary_usage).map_err(JournalCodecError::new)?,
                )
                .map_err(JournalCodecError::new)?;
                if let Some((before, after)) = accounting {
                    checkpoint = checkpoint
                        .with_accounting(before, after)
                        .map_err(JournalCodecError::new)?;
                }
                checkpoint
                    .validate_profile()
                    .map_err(JournalCodecError::new)?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::ContextCheckpoint(checkpoint),
                ))
            },
            WireRecord::MessageReset { reset } => Ok((
                None,
                JournalRecord::MessageReset(MessageReset::try_from(reset)?),
            )),
            WireRecord::MessageSegment { segment } => Ok((
                None,
                JournalRecord::MessageSegment(MessageSegment::try_from(segment)?),
            )),
            WireRecord::MessageEnded {
                final_segment,
                ended,
            } => Ok((
                None,
                JournalRecord::MessageEnded(MessageTerminal::new(
                    final_segment.map(MessageSegment::try_from).transpose()?,
                    MessageEnded::try_from(ended)?,
                )),
            )),
        }
    }
}

impl WireRecord {
    pub(in super::super) fn decode_in_session(
        self,
        child: Option<SessionId>,
    ) -> Result<(Option<JournalSequence>, JournalRecord), JournalCodecError> {
        match self {
            Self::InitialForkSeed(seed) => seed.decode(child.ok_or_else(|| {
                JournalCodecError::new(
                    "initial fork seed requires a preceding descriptor in the same commit",
                )
            })?),
            other => other.try_into(),
        }
    }

    pub(in super::super) fn validate_fork_child(
        child: SessionId,
        seed: &InitialForkSeed,
    ) -> Result<(), JournalCodecError> {
        super::validate_child(child, seed)
    }

    pub(in super::super) fn prepare_initial_fork_seed(
        child: SessionId,
        parent: SessionId,
        source: ForkSource,
        seed: ForkSeed,
        history: Vec<(SessionId, ForkHistoryCoordinate, &SequencedJournalRecord)>,
    ) -> Result<InitialForkSeed, JournalCodecError> {
        super::prepare_seed(child, parent, source, seed, history)
    }
}
