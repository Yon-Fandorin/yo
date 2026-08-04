use serde::{Deserialize, Serialize};

use super::{
    JournalCodecError,
    command::WireCommand,
    correlation,
    correlation::{
        WireBindingCloseReason, WireBindingTransition, WireDetailAvailability,
        WireExchangeDirection, WireExchangeKind, WireResumableStatus, WireVersionedIdentity,
    },
    descriptor::WireSessionDescriptor,
    event::WireEvent,
    message::{WireMessageEnded, WireMessageReset, WireMessageSegment},
};
use crate::{
    AgentEvent, JournalSequence, SessionDescriptor,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, BindingTransition, ContinuationAnchor,
        JournalRecord, MessageEnded, MessageReset, MessageSegment, MessageTerminal,
        SequencedJournalRecord,
    },
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireRecord {
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
    },
    BackendBindingClosed {
        journal_sequence: u64,
        epoch: u64,
        reason: WireBindingCloseReason,
    },
    BackendRequestAccepted {
        journal_sequence: u64,
        epoch: u64,
        turn_id: u64,
        operation_id: String,
        exchange_sequence: u64,
        request_identity: WireVersionedIdentity,
    },
    BackendResumableOutcome {
        journal_sequence: u64,
        epoch: u64,
        turn_id: u64,
        accepted_request_sequence: u64,
        status: WireResumableStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome_identity: Option<WireVersionedIdentity>,
    },
    ContinuationAnchor {
        journal_sequence: u64,
        epoch: u64,
        accepted_request_sequence: u64,
        resumable_outcome_sequence: u64,
        journal_boundary: u64,
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
                transition: WireBindingTransition {
                    mode: binding.transition().mode().into(),
                    cache: binding.transition().cache().into(),
                    source_anchor_sequence: binding
                        .transition()
                        .source_anchor_sequence()
                        .map(JournalSequence::get),
                },
            },
            JournalRecord::BackendBindingClosed(binding) => Self::BackendBindingClosed {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: binding.epoch(),
                reason: binding.reason().into(),
            },
            JournalRecord::BackendRequestAccepted(request) => Self::BackendRequestAccepted {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: request.epoch(),
                turn_id: request.turn_id().get().get(),
                operation_id: request.operation_id().as_uuid().to_string(),
                exchange_sequence: request.exchange_sequence().get(),
                request_identity: correlation::encode_identity(request.request_identity())?,
            },
            JournalRecord::BackendResumableOutcome(outcome) => Self::BackendResumableOutcome {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: outcome.epoch(),
                turn_id: outcome.turn_id().get().get(),
                accepted_request_sequence: outcome.accepted_request_sequence().get(),
                status: WireResumableStatus::Completed,
                outcome_identity: outcome
                    .outcome_identity()
                    .map(correlation::encode_identity)
                    .transpose()?,
            },
            JournalRecord::ContinuationAnchor(anchor) => Self::ContinuationAnchor {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: anchor.epoch(),
                accepted_request_sequence: anchor.accepted_request_sequence().get(),
                resumable_outcome_sequence: anchor.resumable_outcome_sequence().get(),
                journal_boundary: anchor.journal_boundary().get(),
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
                        BindingTransition::new(
                            transition.mode.into(),
                            transition.cache.into(),
                            transition
                                .source_anchor_sequence
                                .map(|value| correlation::sequence(value, "source_anchor_sequence"))
                                .transpose()?,
                        ),
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
                turn_id,
                operation_id,
                exchange_sequence,
                request_identity,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                        epoch,
                        correlation::turn_id(turn_id)?,
                        correlation::operation_id(operation_id)?,
                        correlation::sequence(exchange_sequence, "exchange_sequence")?,
                        correlation::decode_identity(request_identity)?,
                    )),
                ))
            },
            WireRecord::BackendResumableOutcome {
                journal_sequence,
                epoch,
                turn_id,
                accepted_request_sequence,
                status: WireResumableStatus::Completed,
                outcome_identity,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                        epoch,
                        correlation::turn_id(turn_id)?,
                        correlation::sequence(
                            accepted_request_sequence,
                            "accepted_request_sequence",
                        )?,
                        outcome_identity
                            .map(correlation::decode_identity)
                            .transpose()?,
                    )),
                ))
            },
            WireRecord::ContinuationAnchor {
                journal_sequence,
                epoch,
                accepted_request_sequence,
                resumable_outcome_sequence,
                journal_boundary,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
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
                    )),
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

fn required_journal_sequence(
    entry: &SequencedJournalRecord,
) -> Result<JournalSequence, JournalCodecError> {
    entry.journal_sequence().ok_or_else(|| {
        JournalCodecError::new("semantic Journal record is missing journal_sequence")
    })
}
