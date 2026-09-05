//! Reusable transport mechanisms for delegated Agent Backends.

mod jsonrpc;
mod readiness;
mod stdio_jsonl;

pub use jsonrpc::JsonRpcMailbox;
pub use readiness::{Readiness, ReadyReceiver};
pub use stdio_jsonl::{
    DEFAULT_MAX_JSONL_MESSAGE_BYTES, JsonMessagePeer, JsonlPoll, StdioJsonlConfig, StdioJsonlPeer,
};
