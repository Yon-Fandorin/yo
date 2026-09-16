use super::{
    super::{valid_schema, valid_value},
    budget::MAX_REPLAY_TEXT_BYTES,
    private::{ProviderPrivateReplayEnvelope, is_valid_envelope},
};
use crate::ModelInputPart;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelReplayRole {
    System,
    Developer,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelReplayItem {
    /// Ordered image-bearing user content with immutable admitted PNG snapshots.
    MultimodalUser {
        parts: Vec<ModelInputPart>,
    },
    Message {
        role: ModelReplayRole,
        content: String,
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
        envelope: ProviderPrivateReplayEnvelope,
    },
}

impl ModelReplayItem {
    fn is_valid(&self) -> bool {
        match self {
            Self::MultimodalUser { parts } => ModelInputPart::validate_user_parts(parts).is_ok(),
            Self::Message {
                role,
                content,
                refusal,
            } => {
                content.len() <= MAX_REPLAY_TEXT_BYTES
                    && refusal.as_ref().is_none_or(|refusal| {
                        *role == ModelReplayRole::Assistant
                            && refusal.len() <= MAX_REPLAY_TEXT_BYTES
                    })
            },
            Self::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                valid_value(call_id)
                    && valid_schema(name)
                    && arguments.len() <= MAX_REPLAY_TEXT_BYTES
            },
            Self::FunctionCallOutput { call_id, output } => {
                valid_value(call_id) && output.len() <= MAX_REPLAY_TEXT_BYTES
            },
            Self::ProviderPrivateAssistant { envelope } => is_valid_envelope(envelope),
        }
    }
}

pub(super) fn is_valid_item(item: &ModelReplayItem) -> bool {
    item.is_valid()
}
