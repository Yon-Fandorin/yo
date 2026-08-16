use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
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
};
use crate::{
    AgentEvent, JournalSequence, KimiAssistantMessage, KimiAssistantToolCall, ModelReplayContract,
    ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool, SessionDescriptor,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, BindingTransition, ContinuationAnchor,
        JournalRecord, MessageEnded, MessageReset, MessageSegment, MessageTerminal,
        ModelReplayDeltaRecord, SequencedJournalRecord,
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
        turn_id: u64,
        operation_id: String,
        exchange_sequence: u64,
        request_identity: WireVersionedIdentity,
    },
    ModelReplayDelta {
        journal_sequence: u64,
        epoch: u64,
        turn_id: u64,
        accepted_request_sequence: u64,
        #[serde(flatten)]
        replay: WireModelReplayDelta,
    },
    BackendResumableOutcome {
        journal_sequence: u64,
        epoch: u64,
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireModelReplayDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contract: Option<WireModelReplayContract>,
    items: Vec<WireModelReplayItem>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireModelReplayContract {
    system_prompt: String,
    tools: Vec<WireModelReplayTool>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireModelReplayTool {
    name: String,
    description: String,
    schema_version: String,
    parameters: Value,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireModelReplayItem {
    Message {
        role: WireModelReplayRole,
        content: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_refusal"
        )]
        refusal: Option<String>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
    ProviderPrivateAssistant {
        schema: String,
        binding_epoch: u64,
        message: WireKimiAssistantMessage,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireKimiAssistantMessage {
    role: String,
    reasoning_content: String,
    content: WireRequiredNullableString,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_tool_calls",
        skip_serializing_if = "Option::is_none"
    )]
    tool_calls: Option<Vec<WireKimiAssistantToolCall>>,
}

#[derive(Deserialize, Serialize)]
#[serde(transparent)]
struct WireRequiredNullableString(Option<String>);

fn deserialize_optional_tool_calls<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<WireKimiAssistantToolCall>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<WireKimiAssistantToolCall>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireKimiAssistantToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireKimiAssistantFunction,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireKimiAssistantFunction {
    name: String,
    arguments: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireModelReplayRole {
    System,
    Developer,
    User,
    Assistant,
}

fn deserialize_optional_refusal<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
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
                turn_id: request.turn_id().get().get(),
                operation_id: request.operation_id().as_uuid().to_string(),
                exchange_sequence: request.exchange_sequence().get(),
                request_identity: correlation::encode_identity(request.request_identity())?,
            },
            JournalRecord::ModelReplayDelta(replay) => Self::ModelReplayDelta {
                journal_sequence: required_journal_sequence(entry)?.get(),
                epoch: replay.epoch(),
                turn_id: replay.turn_id().get().get(),
                accepted_request_sequence: replay.accepted_request_sequence().get(),
                replay: encode_model_replay(replay.delta(), replay.epoch()),
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
                replay_delta_sequence: outcome.replay_delta_sequence().map(JournalSequence::get),
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
                        BindingTransition::new(
                            transition.mode.into(),
                            transition.cache.into(),
                            transition
                                .source_anchor_sequence
                                .map(|value| correlation::sequence(value, "source_anchor_sequence"))
                                .transpose()?,
                        ),
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
            WireRecord::ModelReplayDelta {
                journal_sequence,
                epoch,
                turn_id,
                accepted_request_sequence,
                replay,
            } => {
                correlation::positive(epoch, "epoch")?;
                Ok((
                    Some(correlation::sequence(journal_sequence, "journal_sequence")?),
                    JournalRecord::ModelReplayDelta(ModelReplayDeltaRecord::new(
                        epoch,
                        correlation::turn_id(turn_id)?,
                        correlation::sequence(
                            accepted_request_sequence,
                            "accepted_request_sequence",
                        )?,
                        decode_model_replay(replay, epoch)?,
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
                replay_delta_sequence,
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
                        replay_delta_sequence
                            .map(|value| correlation::sequence(value, "replay_delta_sequence"))
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

fn encode_model_replay(replay: &ModelReplayDelta, epoch: u64) -> WireModelReplayDelta {
    WireModelReplayDelta {
        contract: replay.contract().map(|contract| WireModelReplayContract {
            system_prompt: contract.system_prompt().to_owned(),
            tools: contract
                .tools()
                .iter()
                .map(|tool| WireModelReplayTool {
                    name: tool.name().to_owned(),
                    description: tool.description().to_owned(),
                    schema_version: tool.schema_version().to_owned(),
                    parameters: tool.parameters().clone(),
                })
                .collect(),
        }),
        items: replay
            .items()
            .iter()
            .map(|item| match item {
                ModelReplayItem::Message {
                    role,
                    content,
                    refusal,
                } => WireModelReplayItem::Message {
                    role: (*role).into(),
                    content: content.clone(),
                    refusal: refusal.clone(),
                },
                ModelReplayItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                } => WireModelReplayItem::FunctionCall {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                },
                ModelReplayItem::FunctionCallOutput { call_id, output } => {
                    WireModelReplayItem::FunctionCallOutput {
                        call_id: call_id.clone(),
                        output: output.clone(),
                    }
                },
                ModelReplayItem::ProviderPrivateAssistant { schema, message } => {
                    WireModelReplayItem::ProviderPrivateAssistant {
                        schema: schema.clone(),
                        binding_epoch: epoch,
                        message: WireKimiAssistantMessage {
                            role: "assistant".to_owned(),
                            reasoning_content: message.reasoning_content().to_owned(),
                            content: WireRequiredNullableString(
                                message.content().map(str::to_owned),
                            ),
                            tool_calls: (!message.tool_calls().is_empty()).then(|| {
                                message
                                    .tool_calls()
                                    .iter()
                                    .map(|call| WireKimiAssistantToolCall {
                                        id: call.id().to_owned(),
                                        kind: "function".to_owned(),
                                        function: WireKimiAssistantFunction {
                                            name: call.name().to_owned(),
                                            arguments: call.arguments().to_owned(),
                                        },
                                    })
                                    .collect()
                            }),
                        },
                    }
                },
            })
            .collect(),
    }
}

fn decode_model_replay(
    wire: WireModelReplayDelta,
    epoch: u64,
) -> Result<ModelReplayDelta, JournalCodecError> {
    let contract = wire.contract.map(|contract| {
        ModelReplayContract::new(
            contract.system_prompt,
            contract
                .tools
                .into_iter()
                .map(|tool| {
                    ModelReplayTool::new(
                        tool.name,
                        tool.description,
                        tool.schema_version,
                        tool.parameters,
                    )
                })
                .collect(),
        )
    });
    let items = wire
        .items
        .into_iter()
        .map(|item| Ok(match item {
            WireModelReplayItem::Message {
                role,
                content,
                refusal,
            } => ModelReplayItem::Message {
                role: role.into(),
                content,
                refusal,
            },
            WireModelReplayItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => ModelReplayItem::FunctionCall {
                call_id,
                name,
                arguments,
            },
            WireModelReplayItem::FunctionCallOutput { call_id, output } => {
                ModelReplayItem::FunctionCallOutput { call_id, output }
            },
            WireModelReplayItem::ProviderPrivateAssistant {
                schema,
                binding_epoch,
                message,
            } => {
                let tool_calls = match message.tool_calls {
                    Some(tool_calls) if tool_calls.is_empty() => {
                        return Err(JournalCodecError::new(
                            "present Kimi private assistant tool_calls must be nonempty",
                        ));
                    },
                    Some(tool_calls) => tool_calls,
                    None => Vec::new(),
                };
                if binding_epoch != epoch
                    || message.role != "assistant"
                    || tool_calls.iter().any(|call| call.kind != "function")
                {
                    return Err(JournalCodecError::new(
                        "Kimi private assistant does not match its replay epoch or closed shape",
                    ));
                }
                ModelReplayItem::ProviderPrivateAssistant {
                    schema,
                    message: KimiAssistantMessage::new(
                        message.reasoning_content,
                        message.content.0,
                        tool_calls
                            .into_iter()
                            .map(|call| {
                                KimiAssistantToolCall::new(
                                    call.id,
                                    call.function.name,
                                    call.function.arguments,
                                )
                            })
                            .collect(),
                    ),
                }
            },
        }))
        .collect::<Result<Vec<_>, JournalCodecError>>()?;
    let delta = ModelReplayDelta::new(contract, items);
    delta.validate().map_err(JournalCodecError::new)?;
    Ok(delta)
}

impl From<ModelReplayRole> for WireModelReplayRole {
    fn from(value: ModelReplayRole) -> Self {
        match value {
            ModelReplayRole::System => Self::System,
            ModelReplayRole::Developer => Self::Developer,
            ModelReplayRole::User => Self::User,
            ModelReplayRole::Assistant => Self::Assistant,
        }
    }
}

impl From<WireModelReplayRole> for ModelReplayRole {
    fn from(value: WireModelReplayRole) -> Self {
        match value {
            WireModelReplayRole::System => Self::System,
            WireModelReplayRole::Developer => Self::Developer,
            WireModelReplayRole::User => Self::User,
            WireModelReplayRole::Assistant => Self::Assistant,
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
