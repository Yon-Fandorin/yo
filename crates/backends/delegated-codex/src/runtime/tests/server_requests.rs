use serde_json::{Value, json};
#[cfg(test)]
use yo_backend::transport::JsonMessagePeer;
#[cfg(test)]
use yo_core::interview::CAPTURE_LIMIT;
#[cfg(test)]
use yo_core::interview::Capture;
#[cfg(test)]
use yo_core::interview::RECOVERY_UNAVAILABLE_RECEIPT_PREFIX;
use yo_core::{
    ActivityApproval, ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse,
    ActivityUpdate, AgentCommand, AgentEvent, AgentRuntime, ApprovalDecision, BackendFailureKind,
    RequestId, RuntimePoll, UserInput,
};

mod approval;
mod input;
mod lifecycle;
mod limits;
