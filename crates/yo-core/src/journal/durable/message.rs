use std::{collections::BTreeMap, time::Duration};

use super::super::codec::{JournalRecord, MessageOutcome, MessageSegmenter, MessageStream};
use crate::{ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate};

#[derive(Debug, Default)]
pub(super) struct MessageTracker {
    messages: BTreeMap<ActivityRef, MessageSegmenter>,
}

impl MessageTracker {
    pub(super) fn start(&mut self, activity: ActivityRef, kind: ActivityKind) -> bool {
        self.messages
            .insert(
                activity,
                MessageSegmenter::new(activity, MessageStream::for_activity(kind)),
            )
            .is_none()
    }

    pub(super) fn update(
        &mut self,
        activity: ActivityRef,
        update: &ActivityUpdate,
        now: Duration,
    ) -> Option<Vec<JournalRecord>> {
        let message = self.messages.get_mut(&activity)?;
        let segments = match update {
            ActivityUpdate::TextDelta(text) => message.push_text(text, now),
            ActivityUpdate::TextSnapshot(text) => message.replace_text(text, now),
        };
        Some(
            segments
                .into_iter()
                .map(JournalRecord::MessageSegment)
                .collect(),
        )
    }

    pub(super) fn finish(
        &mut self,
        activity: ActivityRef,
        outcome: &ActivityOutcome,
    ) -> Option<JournalRecord> {
        let mut message = self.messages.remove(&activity)?;
        Some(message.finish(match outcome {
            ActivityOutcome::Completed => MessageOutcome::Completed,
            ActivityOutcome::Interrupted => MessageOutcome::Interrupted,
            ActivityOutcome::Failed(failure) => MessageOutcome::Failed(failure.clone()),
        }))
    }

    pub(super) fn flush_due(&mut self, now: Duration) -> Vec<JournalRecord> {
        self.messages
            .values_mut()
            .filter_map(|message| message.flush_due(now))
            .collect()
    }

    pub(super) fn flush_boundaries(&mut self, except: Option<ActivityRef>) -> Vec<JournalRecord> {
        self.messages
            .iter_mut()
            .filter(|(activity, _)| Some(**activity) != except)
            .filter_map(|(_, message)| message.flush_boundary())
            .collect()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.messages.len()
    }
}
