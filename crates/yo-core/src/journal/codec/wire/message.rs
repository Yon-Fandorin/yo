use serde::{Deserialize, Serialize};

use super::{JournalCodecError, event::WireOutcome, identity::WireActivityRef};
use crate::{
    ActivityRef,
    journal::codec::{MessageEnded, MessageOutcome, MessageSegment, MessageStream},
};

#[derive(Deserialize, Serialize)]
pub(super) struct WireMessageSegment {
    activity: WireActivityRef,
    stream: WireMessageStream,
    index: u64,
    text: String,
}

#[derive(Deserialize, Serialize)]
pub(super) struct WireMessageEnded {
    activity: WireActivityRef,
    stream: WireMessageStream,
    outcome: WireOutcome,
    segment_count: u64,
    utf8_bytes: u64,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireMessageStream {
    Agent,
    ToolOutput,
}

impl From<&MessageSegment> for WireMessageSegment {
    fn from(segment: &MessageSegment) -> Self {
        Self {
            activity: WireActivityRef::from(segment.activity()),
            stream: WireMessageStream::from(segment.stream()),
            index: segment.index(),
            text: segment.text().to_owned(),
        }
    }
}

impl TryFrom<WireMessageSegment> for MessageSegment {
    type Error = JournalCodecError;

    fn try_from(segment: WireMessageSegment) -> Result<Self, Self::Error> {
        if segment.index == 0 {
            return Err(JournalCodecError::new(
                "MessageSegment index must be positive",
            ));
        }
        if segment.text.is_empty() {
            return Err(JournalCodecError::new(
                "MessageSegment text must not be empty",
            ));
        }
        let stream = MessageStream::from(segment.stream);
        if segment.text.len() > stream.segment_limit() {
            return Err(JournalCodecError::new(
                "MessageSegment exceeds its UTF-8 byte bound",
            ));
        }
        Ok(Self::new(
            ActivityRef::try_from(segment.activity)?,
            stream,
            segment.index,
            segment.text,
        ))
    }
}

impl From<&MessageEnded> for WireMessageEnded {
    fn from(ended: &MessageEnded) -> Self {
        Self {
            activity: WireActivityRef::from(ended.activity()),
            stream: WireMessageStream::from(ended.stream()),
            outcome: WireOutcome::from(ended.outcome()),
            segment_count: ended.segment_count(),
            utf8_bytes: ended.utf8_bytes(),
        }
    }
}

impl TryFrom<WireMessageEnded> for MessageEnded {
    type Error = JournalCodecError;

    fn try_from(ended: WireMessageEnded) -> Result<Self, Self::Error> {
        Ok(Self::new(
            ActivityRef::try_from(ended.activity)?,
            MessageStream::from(ended.stream),
            MessageOutcome::from(ended.outcome),
            ended.segment_count,
            ended.utf8_bytes,
        ))
    }
}

impl From<&MessageOutcome> for WireOutcome {
    fn from(outcome: &MessageOutcome) -> Self {
        match outcome {
            MessageOutcome::Completed => Self::Completed,
            MessageOutcome::Interrupted => Self::Interrupted,
            MessageOutcome::Failed(message) => Self::Failed {
                message: message.clone(),
            },
        }
    }
}

impl From<WireOutcome> for MessageOutcome {
    fn from(outcome: WireOutcome) -> Self {
        match outcome {
            WireOutcome::Completed => Self::Completed,
            WireOutcome::Interrupted => Self::Interrupted,
            WireOutcome::Failed { message } => Self::Failed(message),
        }
    }
}

impl From<MessageStream> for WireMessageStream {
    fn from(stream: MessageStream) -> Self {
        match stream {
            MessageStream::Agent => Self::Agent,
            MessageStream::ToolOutput => Self::ToolOutput,
        }
    }
}

impl From<WireMessageStream> for MessageStream {
    fn from(stream: WireMessageStream) -> Self {
        match stream {
            WireMessageStream::Agent => Self::Agent,
            WireMessageStream::ToolOutput => Self::ToolOutput,
        }
    }
}
