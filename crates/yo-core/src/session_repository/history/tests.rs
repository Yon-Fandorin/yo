use std::num::NonZeroU64;

use super::*;
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent,
    JournalSequence, TranscriptRecord, TurnId, TurnRef,
    journal::codec::{
        JournalCommit, JournalCommitKind, JournalRecord, MessageEnded, MessageOutcome,
        MessageReset, MessageSegment, MessageStream, MessageTerminal, SequencedJournalRecord,
        encode,
    },
    session_repository::{
        DurableRecord, RecordDiscovery, RepositoryEntry, RepositoryError, RepositorySequence,
        StoredSession,
    },
};

#[derive(Debug, Default)]
struct MemoryReader {
    entries: Vec<RepositoryEntry>,
    missing: bool,
}

impl StoredSessionReader for MemoryReader {
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        Ok(Vec::new())
    }

    fn read_session(
        &self,
        _session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        Ok(if self.missing {
            StoredSessionSnapshot::Missing
        } else {
            StoredSessionSnapshot::Present(self.entries.clone())
        })
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

fn session() -> SessionId {
    crate::fixture_session(1)
}

fn activity() -> ActivityRef {
    ActivityRef::new(
        TurnRef::new(session(), TurnId::new(NonZeroU64::new(2).unwrap())),
        ActivityId::new(NonZeroU64::new(3).unwrap()),
    )
}

fn record(commit: &JournalCommit) -> DurableRecord {
    let record = match commit.kind() {
        JournalCommitKind::Incremental => DurableRecord::incremental(encode(commit).unwrap()),
        JournalCommitKind::Snapshot => DurableRecord::snapshot(encode(commit).unwrap()),
    };
    record
        .with_journal_cutoff(commit.journal_cutoff())
        .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session())))
}

fn reader(commits: &[JournalCommit]) -> MemoryReader {
    MemoryReader {
        entries: commits
            .iter()
            .enumerate()
            .map(|(index, commit)| {
                RepositoryEntry::new(
                    RepositorySequence::new(u64::try_from(index).unwrap() + 1),
                    record(commit),
                )
            })
            .collect(),
        missing: false,
    }
}

fn started(sequence: u64) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity: activity(),
            kind: ActivityKind::AgentMessage,
        }),
    )
}

fn finished(sequence: u64, outcome: ActivityOutcome) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity: activity(),
            outcome,
        }),
    )
}

// 여러 physical segment와 그 앞의 오래된 revision은 저장 최적화 경계일 뿐이므로,
// replacement revision의 최종 text snapshot 하나와 종료 event만 frontend에 전달한다.
#[test]
fn coalesces_segments_and_superseded_revisions() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "old".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(4),
            JournalRecord::MessageReset(MessageReset::new(activity(), MessageStream::Agent, 2)),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(5),
            JournalRecord::MessageSegment(MessageSegment::for_revision(
                activity(),
                MessageStream::Agent,
                2,
                1,
                "new ".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(6),
            JournalRecord::MessageEnded(MessageTerminal::new(
                Some(MessageSegment::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    2,
                    "answer".to_owned(),
                )),
                MessageEnded::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    MessageOutcome::Completed,
                    2,
                    10,
                ),
            )),
        ),
        finished(7, ActivityOutcome::Completed),
    ]);

    let history = read_stored_session(&reader(&[commit]), session()).unwrap();
    let updates = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update, .. }) => {
                Some(update)
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        updates,
        [&ActivityUpdate::TextSnapshot("new answer".to_owned())]
    );
    assert!(history.discovery_consistent());
    assert_eq!(history.recovery(), StoredSessionRecovery::NotRequired);
    assert_eq!(history.continuity(), StoredSessionContinuity::NotObservable);
}

// 이전 revision에 text가 있더라도 다음 revision의 zero-byte terminal은 "빈 답변"이라는
// authoritative snapshot이므로, 저장된 Chat에 오래된 text를 남기지 않습니다.
#[test]
fn empty_terminal_revision_clears_superseded_text() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "superseded".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(4),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(5, ActivityOutcome::Completed),
    ]);

    let history = read_stored_session(&reader(&[commit]), session()).unwrap();
    let updates = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update, .. }) => {
                Some(update)
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(updates, [&ActivityUpdate::TextSnapshot(String::new())]);
}

// crash가 열린 message의 terminal을 쓰기 전에 멈추면 recovery seal을 버리지 않고
// 마지막 text 뒤에 interrupted ActivityFinished를 붙여 완료된 대화처럼 보이지 않게 한다.
#[test]
fn projects_an_open_message_as_explicitly_interrupted() {
    let descriptor = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let open = JournalCommit::incremental(vec![
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "partial".to_owned(),
            )),
        ),
    ]);

    let history = read_stored_session(&reader(&[descriptor, open]), session()).unwrap();

    assert_eq!(history.recovery(), StoredSessionRecovery::Interrupted);
    assert!(matches!(
        history.records().last(),
        Some(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                outcome: ActivityOutcome::Interrupted,
                ..
            }
        ))
    ));
}

// message terminal과 semantic ActivityFinished의 outcome이 다르면 손상된 history를 일부
// 출력하지 않고 두 authority가 충돌한다는 복구 오류로 거부한다.
#[test]
fn rejects_conflicting_message_and_activity_outcomes() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::new(
                    activity(),
                    MessageStream::Agent,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(4, ActivityOutcome::Interrupted),
    ]);

    let error = read_stored_session(&reader(&[commit]), session()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("conflicting message and activity outcomes")
    );
}

// envelope의 snapshot 표기와 내부 commit 종류가 다르면 부분 투영하지 않고 정확한
// physical repository sequence가 포함된 경계 오류를 반환한다.
#[test]
fn rejects_an_envelope_kind_mismatch() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::new(
                    activity(),
                    MessageStream::Agent,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(4, ActivityOutcome::Completed),
    ]);
    let mismatched = DurableRecord::incremental(encode(&commit).unwrap())
        .with_journal_cutoff(commit.journal_cutoff())
        .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session())));
    let reader = MemoryReader {
        entries: vec![RepositoryEntry::new(RepositorySequence::new(7), mismatched)],
        missing: false,
    };

    let error = read_stored_session(&reader, session()).unwrap_err();

    assert!(error.to_string().contains("repository sequence 7"));
    assert!(error.to_string().contains("envelope does not match"));
}

// 파일 자체가 없는 경우와 파일은 있지만 complete envelope가 하나도 없는 경우는 복구
// 조치가 다르므로 direct read error에서도 각각 NotFound와 Incomplete로 구분합니다.
#[test]
fn distinguishes_missing_and_incomplete_physical_history() {
    let missing = MemoryReader {
        entries: Vec::new(),
        missing: true,
    };
    let incomplete = MemoryReader::default();

    assert!(matches!(
        read_stored_session(&missing, session()),
        Err(StoredSessionReadError::NotFound { .. })
    ));
    assert!(matches!(
        read_stored_session(&incomplete, session()),
        Err(StoredSessionReadError::Incomplete { .. })
    ));
}
