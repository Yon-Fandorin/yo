use std::{
    fmt,
    sync::{Arc, RwLock},
};

use super::{JournalSequence, SessionJournalState, read_state};
use crate::RequestTraceEntry;

const READ_LIMIT: usize = 256;

/// Read-only access to the payload-free Request correlation trace of one live Session.
#[derive(Clone)]
pub struct RequestTraceReader {
    pub(super) state: Arc<RwLock<SessionJournalState>>,
}

impl fmt::Debug for RequestTraceReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestTraceReader")
            .finish_non_exhaustive()
    }
}

impl RequestTraceReader {
    /// Copies the next bounded group of correlation records after `sequence`.
    #[must_use]
    pub fn read_after(&self, sequence: Option<JournalSequence>) -> RequestTraceSlice {
        let state = read_state(&self.state);
        let head = state
            .entries
            .iter()
            .rev()
            .find_map(project)
            .map(|entry| entry.sequence());
        let start = sequence.map_or(0, |sequence| {
            state
                .entries
                .partition_point(|entry| entry.sequence() <= sequence)
        });
        let entries = state
            .entries
            .iter()
            .skip(start)
            .filter_map(project)
            .take(READ_LIMIT)
            .collect();
        RequestTraceSlice { head, entries }
    }
}

fn project(entry: &super::JournalEntry) -> Option<RequestTraceEntry> {
    crate::request_trace::project_live(entry.sequence(), entry.record())
}

/// One bounded live Request-trace page and its observed head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestTraceSlice {
    head: Option<JournalSequence>,
    entries: Vec<RequestTraceEntry>,
}

impl RequestTraceSlice {
    #[must_use]
    pub const fn head(&self) -> Option<JournalSequence> {
        self.head
    }

    #[must_use]
    pub fn entries(&self) -> &[RequestTraceEntry] {
        &self.entries
    }

    #[must_use]
    pub fn into_entries(self) -> Vec<RequestTraceEntry> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::{
        super::{JournalEntry, SemanticRecord, SessionJournal, write_state},
        READ_LIMIT,
    };
    use crate::{
        AgentCommand, AgentEvent, BackendBindingEvidence, BackendIdentity, BackendRequestEvidence,
        InputSubmission, JournalSequence, SubmissionId, TurnId, TurnRef, UserInput,
        journal::codec::{BackendBindingClosed, BindingCloseReason},
    };

    // live reader는 일반 command/event를 건너뛰고, reader 생성 뒤 추가된 상관 레코드도
    // JournalSequence 커서로 중복 없이 이어 읽습니다. sequence가 비연속이어도 256개
    // 경계 다음 page가 빠지지 않아, 저장 복구 뒤의 희소한 번호도 같은 방식으로 읽습니다.
    #[test]
    fn reader_pages_only_payload_free_live_correlation_records() {
        let session_id = crate::fixture_session(1);
        let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::MIN));
        let mut journal = SessionJournal::new();
        let reader = journal.request_trace_reader();
        journal.append_initial_binding(
            AgentCommand::CreateSession { session_id },
            &[AgentEvent::SessionCreated { session_id }],
            1,
            BackendBindingEvidence::new(
                "codex-app-server",
                "test",
                BackendIdentity::new("binding/v1", "binding"),
                BackendIdentity::new("model/v1", "model"),
                BackendIdentity::new("session/v1", "session"),
                crate::ContinuationStrategy::BackendManagedState,
            ),
        );

        let first = reader.read_after(None);
        assert_eq!(first.entries().len(), 1);
        let cursor = first.entries()[0].sequence();
        let submission_id: SubmissionId = "10000000-0000-4000-8000-000000000031".parse().unwrap();
        let submission = InputSubmission::new(submission_id, UserInput::new("inspect"));
        journal.append_accepted_submission(
            AgentCommand::StartTurn {
                turn,
                input: submission.input().clone(),
            },
            submission.id(),
            &[AgentEvent::TurnStarted { turn }],
            1,
            BackendRequestEvidence::new(
                "payload/v1",
                BackendIdentity::new("exchange/v1", "exchange"),
                BackendIdentity::new("request/v1", "request"),
            ),
        );

        let second = reader.read_after(Some(cursor));
        assert_eq!(second.entries().len(), 2);
        assert!(
            second
                .entries()
                .windows(2)
                .all(|pair| pair[0].sequence() < pair[1].sequence())
        );
        assert_eq!(
            second.head(),
            second.entries().last().map(|entry| entry.sequence())
        );

        write_state(&reader.state).entries = (0_u64..257)
            .map(|index| {
                JournalEntry::new(
                    JournalSequence::new(index * 2 + 1),
                    SemanticRecord::BackendBindingClosed(BackendBindingClosed::new(
                        index + 1,
                        BindingCloseReason::Exhausted,
                    )),
                )
            })
            .collect();
        let first_page = reader.read_after(None);
        assert_eq!(first_page.entries().len(), READ_LIMIT);
        let final_page =
            reader.read_after(first_page.entries().last().map(|entry| entry.sequence()));
        assert_eq!(final_page.entries().len(), 1);
        assert_eq!(
            final_page.entries()[0].sequence(),
            JournalSequence::new(513)
        );
    }
}
