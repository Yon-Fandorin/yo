mod config;
mod lifecycle;
mod peer;
mod reader;

pub use config::{DEFAULT_MAX_JSONL_MESSAGE_BYTES, StdioJsonlConfig};
pub use peer::{JsonMessagePeer, JsonlPoll, StdioJsonlPeer};

#[cfg(test)]
mod tests;
