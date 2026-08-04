use std::collections::BTreeMap;

use crate::{
    ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, TranscriptRecord,
    journal::codec::{JournalRecord, MessageOutcome, MessageSegment, RecoveredJournal},
};

#[derive(Debug, Default)]
struct HistoryNormalizer {
    output: Vec<TranscriptRecord>,
    messages: BTreeMap<ActivityRef, RecoveredMessage>,
    dirty_order: Vec<ActivityRef>,
}

#[derive(Debug, Default)]
struct RecoveredMessage {
    revision: u64,
    text: String,
    dirty: bool,
    terminal: Option<MessageOutcome>,
}

pub(super) fn normalize(recovered: &RecoveredJournal) -> Result<Vec<TranscriptRecord>, String> {
    let mut normalizer = HistoryNormalizer::default();
    for entry in recovered.records() {
        normalizer.observe(entry.record())?;
    }
    if let Some(commit) = recovered.recovery_commit() {
        for entry in commit.records() {
            normalizer.observe_recovery(entry.record())?;
        }
    }
    normalizer.finish()
}

impl HistoryNormalizer {
    fn finish(mut self) -> Result<Vec<TranscriptRecord>, String> {
        self.flush_dirty();
        for (activity, message) in self.messages {
            let Some(MessageOutcome::Interrupted) = message.terminal else {
                return Err(format!(
                    "stored Session ended with an unsealed message activity {activity:?}"
                ));
            };
            self.output.push(TranscriptRecord::EventCommitted(
                AgentEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Interrupted,
                },
            ));
        }
        Ok(self.output)
    }

    fn observe(&mut self, record: &JournalRecord) -> Result<(), String> {
        match record {
            JournalRecord::SessionDescriptor(_) => Ok(()),
            JournalRecord::CommandCommitted(command) => {
                self.flush_dirty();
                self.output.push(TranscriptRecord::CommandCommitted(
                    command.command().clone(),
                ));
                Ok(())
            },
            JournalRecord::EventCommitted(AgentEvent::ActivityStarted { activity, .. }) => {
                self.flush_dirty();
                self.messages.insert(*activity, RecoveredMessage::default());
                self.push_event(record);
                Ok(())
            },
            JournalRecord::EventCommitted(AgentEvent::ActivityFinished { activity, outcome }) => {
                self.flush_dirty();
                let message = self.messages.remove(activity).ok_or_else(|| {
                    format!("stored activity {activity:?} finished before it started")
                })?;
                let terminal = message.terminal.ok_or_else(|| {
                    format!("stored activity {activity:?} has no durable message terminal")
                })?;
                if !outcomes_match(&terminal, outcome) {
                    return Err(format!(
                        "stored activity {activity:?} has conflicting message and activity outcomes"
                    ));
                }
                self.push_event(record);
                Ok(())
            },
            JournalRecord::EventCommitted(_) => {
                self.flush_dirty();
                self.push_event(record);
                Ok(())
            },
            JournalRecord::BackendExchangeObserved(_)
            | JournalRecord::BackendBindingOpened(_)
            | JournalRecord::BackendBindingClosed(_)
            | JournalRecord::BackendRequestAccepted(_)
            | JournalRecord::BackendResumableOutcome(_)
            | JournalRecord::ContinuationAnchor(_) => Ok(()),
            JournalRecord::MessageReset(reset) => {
                let message = self.message_mut(reset.activity())?;
                message.revision = reset.revision();
                message.text.clear();
                self.mark_dirty(reset.activity());
                Ok(())
            },
            JournalRecord::MessageSegment(segment) => self.apply_segment(segment),
            JournalRecord::MessageEnded(terminal) => {
                if let Some(segment) = terminal.final_segment() {
                    self.apply_segment(segment)?;
                }
                let ended = terminal.ended();
                let message = self.message_mut(ended.activity())?;
                if ended.revision() > message.revision {
                    message.revision = ended.revision();
                    message.text.clear();
                    self.mark_dirty(ended.activity());
                }
                self.message_mut(ended.activity())?.terminal = Some(ended.outcome().clone());
                Ok(())
            },
        }
    }

    fn observe_recovery(&mut self, record: &JournalRecord) -> Result<(), String> {
        let JournalRecord::MessageEnded(terminal) = record else {
            return Err("stored recovery produced a non-terminal record".to_owned());
        };
        let ended = terminal.ended();
        self.message_mut(ended.activity())?.terminal = Some(ended.outcome().clone());
        Ok(())
    }

    fn apply_segment(&mut self, segment: &MessageSegment) -> Result<(), String> {
        let message = self.message_mut(segment.activity())?;
        if segment.revision() > message.revision {
            message.revision = segment.revision();
            message.text.clear();
        }
        message.text.push_str(segment.text());
        self.mark_dirty(segment.activity());
        Ok(())
    }

    fn push_event(&mut self, record: &JournalRecord) {
        let JournalRecord::EventCommitted(event) = record else {
            unreachable!("only event records reach the event projection helper");
        };
        self.output
            .push(TranscriptRecord::EventCommitted(event.clone()));
    }

    fn message_mut(&mut self, activity: ActivityRef) -> Result<&mut RecoveredMessage, String> {
        self.messages
            .get_mut(&activity)
            .ok_or_else(|| format!("stored message {activity:?} has no started activity"))
    }

    fn mark_dirty(&mut self, activity: ActivityRef) {
        let message = self
            .messages
            .get_mut(&activity)
            .expect("the message was validated before marking it dirty");
        if !message.dirty {
            message.dirty = true;
            self.dirty_order.push(activity);
        }
    }

    fn flush_dirty(&mut self) {
        for activity in self.dirty_order.drain(..) {
            let message = self
                .messages
                .get_mut(&activity)
                .expect("a dirty message remains live until its boundary is flushed");
            message.dirty = false;
            self.output.push(TranscriptRecord::EventCommitted(
                AgentEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(message.text.clone()),
                },
            ));
        }
    }
}

fn outcomes_match(message: &MessageOutcome, activity: &ActivityOutcome) -> bool {
    match (message, activity) {
        (MessageOutcome::Completed, ActivityOutcome::Completed)
        | (MessageOutcome::Interrupted, ActivityOutcome::Interrupted) => true,
        (MessageOutcome::Failed(left), ActivityOutcome::Failed(right)) => left == right.message(),
        _ => false,
    }
}
