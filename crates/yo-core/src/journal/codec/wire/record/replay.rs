use std::fmt::{Formatter, Result as FmtResult};

use serde::{
    Deserialize, Serialize,
    de::{IgnoredAny, SeqAccess, Visitor},
};
use serde_json::Value;

use super::super::{JournalCodecError, correlation};
use crate::{
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ProviderPrivateReplayEnvelope, ProviderPrivateReplayPayload,
    journal::codec::{ContextImageLoss, ContextLoss, ContextRetainedGroup},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct WireModelReplayDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contract: Option<WireModelReplayContract>,
    items: Vec<WireModelReplayItem>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct WireModelReplayContract {
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
#[serde(untagged)]
pub(in super::super) enum WireContextRetainedGroup {
    Local(WireLocalRetainedGroup),
    Imported(WireImportedRetainedGroup),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct WireLocalRetainedGroup {
    first_sequence: u64,
    last_sequence: u64,
    items: Vec<WireModelReplayItem>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct WireImportedRetainedGroup {
    profile: String,
    fork_seed_sequence: u64,
    group_index: usize,
    items: Vec<WireModelReplayItem>,
}

impl WireContextRetainedGroup {
    pub(in super::super) fn encode(group: &ContextRetainedGroup, epoch: u64) -> Self {
        if let Some((seed, group_index)) = group.fork_import() {
            let mut private_epochs = group.private_epochs().iter();
            let items = group
                .items()
                .iter()
                .map(|item| {
                    let source_epoch =
                        if matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }) {
                            *private_epochs
                                .next()
                                .expect("validated private epoch cardinality")
                        } else {
                            epoch
                        };
                    encode_model_replay_item(item, source_epoch)
                })
                .collect();
            Self::Imported(WireImportedRetainedGroup {
                profile: "yo.fork-retained-group/v1".into(),
                fork_seed_sequence: seed.get(),
                group_index,
                items,
            })
        } else {
            Self::Local(WireLocalRetainedGroup {
                first_sequence: group.first_sequence().get(),
                last_sequence: group.last_sequence().get(),
                items: group
                    .items()
                    .iter()
                    .map(|item| encode_model_replay_item(item, epoch))
                    .collect(),
            })
        }
    }

    pub(in super::super) fn decode(
        self,
        epoch: u64,
    ) -> Result<ContextRetainedGroup, JournalCodecError> {
        match self {
            Self::Local(group) => ContextRetainedGroup::try_new(
                correlation::sequence(group.first_sequence, "first_sequence")?,
                correlation::sequence(group.last_sequence, "last_sequence")?,
                decode_model_replay_items(group.items, epoch)?,
            )
            .map_err(JournalCodecError::new),
            Self::Imported(group) => {
                if group.profile != "yo.fork-retained-group/v1" || group.items.len() > 4096 {
                    return Err(JournalCodecError::new(
                        "invalid imported checkpoint group profile or item bound",
                    ));
                }
                let mut private_epochs = Vec::new();
                let mut items = Vec::with_capacity(group.items.len());
                for item in group.items {
                    let source_epoch = match &item {
                        WireModelReplayItem::ProviderPrivateAssistant { binding_epoch, .. } => {
                            correlation::positive(*binding_epoch, "imported private epoch")?;
                            private_epochs.push(*binding_epoch);
                            *binding_epoch
                        },
                        _ => epoch,
                    };
                    items.extend(decode_model_replay_items(vec![item], source_epoch)?);
                }
                ContextRetainedGroup::try_imported(
                    correlation::sequence(group.fork_seed_sequence, "fork_seed_sequence")?,
                    group.group_index,
                    items,
                    private_epochs,
                )
                .map_err(JournalCodecError::new)
            },
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct WireContextArtifactReceipt {
    pub(in super::super) profile: String,
    pub(in super::super) content_hash: String,
    pub(in super::super) byte_count: u64,
    pub(in super::super) media_kind: String,
    pub(in super::super) source_context_epoch: u64,
    pub(in super::super) source_journal_sequence: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(in super::super) enum WireContextLoss {
    ImageInputSummarized(ContextImageLoss),
    VisiblePrefixSummarized {
        first_sequence: u64,
        last_sequence: u64,
    },
    ProviderPrivateDropped {
        schema: String,
        present: bool,
        byte_count: u64,
        source_journal_sequence: u64,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(in super::super) enum WireModelReplayItem {
    MultimodalUser {
        #[serde(deserialize_with = "deserialize_multimodal_parts")]
        parts: Vec<crate::ModelInputPart>,
    },
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
        message: ProviderPrivateReplayPayload,
    },
}

fn deserialize_multimodal_parts<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Vec<crate::ModelInputPart>, D::Error> {
    struct PartsVisitor;
    impl<'de> Visitor<'de> for PartsVisitor {
        type Value = Vec<crate::ModelInputPart>;
        fn expecting(&self, f: &mut Formatter<'_>) -> FmtResult {
            f.write_str("one to thirty-three ordered multimodal user parts")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            use serde::de::Error as _;
            let mut parts = Vec::new();
            let mut images = 0_usize;
            let mut png_bytes = 0_usize;
            while parts.len() < 33 {
                let Some(part) = sequence.next_element::<crate::ModelInputPart>()? else {
                    crate::ModelInputPart::validate_user_parts(&parts).map_err(A::Error::custom)?;
                    return Ok(parts);
                };
                match &part {
                    crate::ModelInputPart::Text { text } => {
                        if text.is_empty()
                            || text.len() > 16 * 1024 * 1024
                            || matches!(parts.last(), Some(crate::ModelInputPart::Text { .. }))
                        {
                            return Err(A::Error::custom(
                                "invalid empty, adjacent or over-bound multimodal text",
                            ));
                        }
                    },
                    crate::ModelInputPart::Image { snapshot } => {
                        images += 1;
                        png_bytes = png_bytes
                            .checked_add(snapshot.png().len())
                            .ok_or_else(|| A::Error::custom("multimodal image byte overflow"))?;
                        if images > 16 || png_bytes > crate::InputImageSnapshot::MAX_BYTES {
                            return Err(A::Error::custom(
                                "multimodal image count or byte limit exceeded",
                            ));
                        }
                    },
                }
                parts.push(part);
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom("multimodal part limit exceeded"));
            }
            crate::ModelInputPart::validate_user_parts(&parts).map_err(A::Error::custom)?;
            Ok(parts)
        }
    }
    decoder.deserialize_seq(PartsVisitor)
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

pub(in super::super) fn encode_model_replay(
    replay: &ModelReplayDelta,
    epoch: u64,
) -> WireModelReplayDelta {
    WireModelReplayDelta {
        contract: replay.contract().map(encode_model_replay_contract),
        items: replay
            .items()
            .iter()
            .map(|item| encode_model_replay_item(item, epoch))
            .collect(),
    }
}

pub(in super::super) fn encode_model_replay_contract(
    contract: &ModelReplayContract,
) -> WireModelReplayContract {
    WireModelReplayContract {
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
    }
}

pub(in super::super) fn encode_model_replay_item(
    item: &ModelReplayItem,
    epoch: u64,
) -> WireModelReplayItem {
    match item {
        ModelReplayItem::MultimodalUser { parts } => WireModelReplayItem::MultimodalUser {
            parts: parts.clone(),
        },
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
        ModelReplayItem::ProviderPrivateAssistant { envelope } => {
            WireModelReplayItem::ProviderPrivateAssistant {
                schema: envelope.schema().to_owned(),
                binding_epoch: epoch,
                message: envelope.ordered_payload(),
            }
        },
    }
}

pub(in super::super) fn decode_model_replay(
    wire: WireModelReplayDelta,
    epoch: u64,
) -> Result<ModelReplayDelta, JournalCodecError> {
    let contract = wire.contract.map(decode_model_replay_contract);
    let items = decode_model_replay_items(wire.items, epoch)?;
    let delta = ModelReplayDelta::new(contract, items);
    delta.validate().map_err(JournalCodecError::new)?;
    Ok(delta)
}

pub(in super::super) fn decode_model_replay_contract(
    contract: WireModelReplayContract,
) -> ModelReplayContract {
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
}

pub(in super::super) fn decode_model_replay_items(
    items: Vec<WireModelReplayItem>,
    epoch: u64,
) -> Result<Vec<ModelReplayItem>, JournalCodecError> {
    items
        .into_iter()
        .map(|item| {
            Ok(match item {
                WireModelReplayItem::MultimodalUser { parts } => {
                    crate::ModelInputPart::validate_user_parts(&parts)
                        .map_err(JournalCodecError::new)?;
                    ModelReplayItem::MultimodalUser { parts }
                },
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
                    if binding_epoch != epoch {
                        return Err(JournalCodecError::new(
                            "provider-private assistant does not match its replay epoch",
                        ));
                    }
                    ModelReplayItem::ProviderPrivateAssistant {
                        envelope: ProviderPrivateReplayEnvelope::new(
                            schema,
                            serde_json::to_vec(&message)
                                .expect("decoded provider-private JSON is serializable"),
                        )
                        .map_err(JournalCodecError::new)?,
                    }
                },
            })
        })
        .collect()
}

pub(in super::super) fn encode_context_loss(loss: &ContextLoss) -> WireContextLoss {
    match loss {
        ContextLoss::ImageInputSummarized(loss) => {
            WireContextLoss::ImageInputSummarized(loss.clone())
        },
        ContextLoss::VisiblePrefixSummarized {
            first_sequence,
            last_sequence,
        } => WireContextLoss::VisiblePrefixSummarized {
            first_sequence: first_sequence.get(),
            last_sequence: last_sequence.get(),
        },
        ContextLoss::ProviderPrivateDropped {
            schema,
            byte_count,
            source_journal_sequence,
        } => WireContextLoss::ProviderPrivateDropped {
            schema: schema.clone(),
            present: true,
            byte_count: *byte_count,
            source_journal_sequence: source_journal_sequence.get(),
        },
    }
}

pub(in super::super) fn decode_context_loss(
    loss: WireContextLoss,
) -> Result<ContextLoss, JournalCodecError> {
    match loss {
        WireContextLoss::ImageInputSummarized(loss) => Ok(ContextLoss::ImageInputSummarized(loss)),
        WireContextLoss::VisiblePrefixSummarized {
            first_sequence,
            last_sequence,
        } => ContextLoss::visible_prefix_summarized(
            correlation::sequence(first_sequence, "first_sequence")?,
            correlation::sequence(last_sequence, "last_sequence")?,
        )
        .map_err(JournalCodecError::new),
        WireContextLoss::ProviderPrivateDropped {
            schema,
            present: true,
            byte_count,
            source_journal_sequence,
        } => ContextLoss::provider_private_dropped(
            schema,
            byte_count,
            correlation::sequence(source_journal_sequence, "source_journal_sequence")?,
        )
        .map_err(JournalCodecError::new),
        WireContextLoss::ProviderPrivateDropped { present: false, .. } => Err(
            JournalCodecError::new("provider-private context loss requires present: true"),
        ),
    }
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
