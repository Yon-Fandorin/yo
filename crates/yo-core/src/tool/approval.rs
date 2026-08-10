use sha2::{Digest, Sha256};

use super::{
    registry::ValidatedToolCall,
    schema::{ToolEffect, ToolId},
};
use crate::TurnRef;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolApprovalBinding {
    turn: TurnRef,
    call_id: String,
    tool_id: ToolId,
    argument_digest: [u8; 32],
    effect: ToolEffect,
    execution_host: String,
}

impl ToolApprovalBinding {
    pub fn new(turn: TurnRef, call: &ValidatedToolCall, execution_host: impl Into<String>) -> Self {
        Self {
            turn,
            call_id: call.call_id().to_owned(),
            tool_id: call.definition().id().clone(),
            argument_digest: Sha256::digest(call.normalized_arguments()).into(),
            effect: call.definition().effect(),
            execution_host: execution_host.into(),
        }
    }

    pub const fn turn(&self) -> TurnRef {
        self.turn
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub const fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    pub const fn effect(&self) -> ToolEffect {
        self.effect
    }

    pub fn execution_host(&self) -> &str {
        &self.execution_host
    }

    pub fn argument_digest_hex(&self) -> String {
        self.argument_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn matches(&self, turn: TurnRef, call: &ValidatedToolCall, host: &str) -> bool {
        self.turn == turn
            && self.call_id == call.call_id()
            && self.tool_id == *call.definition().id()
            && self.argument_digest == Sha256::digest(call.normalized_arguments()).as_slice()
            && self.effect == call.definition().effect()
            && self.execution_host == host
    }
}
