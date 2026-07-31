use std::time::Duration;

use super::{
    JournalRecord, MessageEnded, MessageOutcome, MessageSegment, MessageStream, MessageTerminal,
};
use crate::ActivityRef;

const MAX_UNCOMMITTED_AGE: Duration = Duration::from_secs(1);

#[derive(Debug)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "runtime segment capture is owned by a later Slice"
    )
)]
pub(crate) struct MessageSegmenter {
    activity: ActivityRef,
    stream: MessageStream,
    pending: String,
    oldest_pending_at: Option<Duration>,
    segment_count: u64,
    total_bytes: u64,
    ended: bool,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "runtime segment capture is owned by a later Slice"
    )
)]
impl MessageSegmenter {
    pub(crate) fn new(activity: ActivityRef, stream: MessageStream) -> Self {
        Self {
            activity,
            stream,
            pending: String::new(),
            oldest_pending_at: None,
            segment_count: 0,
            total_bytes: 0,
            ended: false,
        }
    }

    pub(crate) fn push_text(&mut self, text: &str, now: Duration) -> Vec<MessageSegment> {
        assert!(!self.ended, "a terminated message cannot accept more text");
        let mut segments = self.flush_due(now).into_iter().collect::<Vec<_>>();
        if text.is_empty() {
            return segments;
        }
        if self.pending.is_empty() {
            self.oldest_pending_at = Some(now);
        }
        self.pending.push_str(text);
        self.total_bytes = self
            .total_bytes
            .checked_add(u64::try_from(text.len()).expect("text length fits u64"))
            .expect("one message cannot contain more than u64 bytes");

        while self.pending.len() >= self.stream.segment_limit() {
            segments.push(self.take_bounded_segment());
            if !self.pending.is_empty() {
                self.oldest_pending_at = Some(now);
            }
        }
        segments
    }

    pub(crate) fn flush_due(&mut self, now: Duration) -> Option<MessageSegment> {
        let due = self.oldest_pending_at.is_some_and(|oldest| {
            now.checked_sub(oldest)
                .is_some_and(|age| age >= MAX_UNCOMMITTED_AGE)
        });
        if due { self.flush_non_empty() } else { None }
    }

    pub(crate) fn flush_boundary(&mut self) -> Option<MessageSegment> {
        self.flush_non_empty()
    }

    pub(crate) fn finish(&mut self, outcome: MessageOutcome) -> JournalRecord {
        assert!(!self.ended, "a message can be terminated only once");
        self.ended = true;
        let final_segment = self.flush_non_empty();
        let ended = MessageEnded::new(
            self.activity,
            self.stream,
            outcome,
            self.segment_count,
            self.total_bytes,
        );
        JournalRecord::MessageEnded(MessageTerminal::new(final_segment, ended))
    }

    fn flush_non_empty(&mut self) -> Option<MessageSegment> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.take_segment(self.pending.len()))
        }
    }

    fn take_bounded_segment(&mut self) -> MessageSegment {
        let mut end = self.stream.segment_limit().min(self.pending.len());
        while !self.pending.is_char_boundary(end) {
            end = end
                .checked_sub(1)
                .expect("the positive segment bound contains a UTF-8 boundary");
        }
        self.take_segment(end)
    }

    fn take_segment(&mut self, end: usize) -> MessageSegment {
        let tail = self.pending.split_off(end);
        let text = std::mem::replace(&mut self.pending, tail);
        self.segment_count = self
            .segment_count
            .checked_add(1)
            .expect("one message cannot contain more than u64 segments");
        if self.pending.is_empty() {
            self.oldest_pending_at = None;
        }
        MessageSegment::new(self.activity, self.stream, self.segment_count, text)
    }
}
