use std::collections::VecDeque;

use yo_core::{
    AgentSession, JournalSequence, RequestTraceReader, TranscriptObservation,
    TranscriptObservationSequence, TranscriptReader,
};
use yo_tui::AgentPoll;

pub(super) struct JournalState {
    transcript: TranscriptReader,
    request_trace: RequestTraceReader,
    cursor: Option<TranscriptObservationSequence>,
    request_cursor: Option<JournalSequence>,
    changed: bool,
}

impl JournalState {
    pub(in crate::agent) fn from_session(session: &AgentSession) -> Self {
        Self {
            transcript: session.transcript_reader(),
            request_trace: session.request_trace_reader(),
            cursor: None,
            request_cursor: None,
            changed: false,
        }
    }

    pub(super) fn initial_binding_record(&self) -> Option<yo_core::RequestTraceRecord> {
        self.request_trace
            .read_after(None)
            .into_entries()
            .into_iter()
            .find_map(|entry| match entry.record() {
                record @ yo_core::RequestTraceRecord::BindingOpened { epoch: 1, .. } => {
                    Some(record.clone())
                },
                _ => None,
            })
    }

    pub(super) fn mark_changed(&mut self) {
        self.changed = true;
    }

    pub(super) fn is_changed(&self) -> bool {
        self.changed
    }

    #[cfg(test)]
    pub(super) fn transcript_head_sequence(&self) -> Option<JournalSequence> {
        self.transcript.head_sequence()
    }

    pub(super) fn drain_into(&mut self, pending: &mut VecDeque<AgentPoll>) {
        if !self.changed {
            return;
        }

        let slice = self.transcript.read_observations_after(self.cursor);
        let head = slice.head();
        for entry in slice.into_entries() {
            self.cursor = Some(entry.sequence());
            pending.push_back(match entry.observation() {
                TranscriptObservation::Durability(durability) => AgentPoll::Durability(*durability),
                TranscriptObservation::Record(record) => AgentPoll::Record(record.clone()),
            });
        }

        let trace = self.request_trace.read_after(self.request_cursor);
        let trace_head = trace.head();
        for entry in trace.into_entries() {
            self.request_cursor = Some(entry.sequence());
            pending.push_back(AgentPoll::RequestTrace(entry));
        }

        self.changed = self.cursor != head || self.request_cursor != trace_head;
    }
}
