use super::{READ_LIMIT, TranscriptRecord};
use crate::{
    AgentCommand, AgentEvent, SessionId,
    journal::{JournalSequence, SessionJournal},
};

fn session(value: u64) -> SessionId {
    crate::fixture_session(value)
}

// Reader를 먼저 만들어 둔 뒤 Worker 역할의 Journal writer가 기록을 추가해도 같은
// 공유 Journal을 바라봐야 TUI와 GUI가 새 기록을 별도 복제본 없이 읽을 수 있다.
#[test]
fn reader_observes_records_appended_after_it_was_created() {
    let session_id = session(1);
    let mut journal = SessionJournal::new();
    let reader = journal.transcript_reader();

    assert!(reader.read_after(None).entries().is_empty());
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );

    let slice = reader.read_after(None);
    assert_eq!(slice.from().map(JournalSequence::get), Some(1));
    assert_eq!(slice.head().map(JournalSequence::get), Some(2));
    assert_eq!(
        slice
            .entries()
            .iter()
            .map(|entry| entry.sequence().get())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        slice.entries()[0].record(),
        &TranscriptRecord::CommandCommitted(AgentCommand::CreateSession { session_id })
    );
}

// 화면이 마지막으로 적용한 sequence를 다시 넘기면 그 다음 기록만 반환해야 재조회나
// 여러 frontend의 서로 다른 읽기 속도 때문에 같은 항목이 중복 표시되지 않는다.
#[test]
fn reads_only_the_suffix_after_each_frontend_cursor() {
    let session_id = session(1);
    let mut journal = SessionJournal::new();
    let reader = journal.transcript_reader();
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    let first = reader.read_after(None);
    let cursor = first.entries().last().unwrap().sequence();

    journal.append_events(&[AgentEvent::SessionCreated { session_id }]);

    let suffix = reader.read_after(Some(cursor));
    assert_eq!(suffix.from().map(JournalSequence::get), Some(3));
    assert_eq!(suffix.head().map(JournalSequence::get), Some(3));
    assert_eq!(suffix.entries().len(), 1);
    assert_eq!(
        suffix.entries()[0].record(),
        &TranscriptRecord::EventCommitted(AgentEvent::SessionCreated { session_id })
    );
}

// 아주 긴 Session을 처음 여는 frontend도 한 번의 read lock으로 전체 기록을 복사하지
// 않고 제한된 묶음씩 따라가야 Worker의 append 기회를 오래 빼앗지 않는다.
#[test]
fn bounds_each_read_and_keeps_the_complete_head_visible() {
    let session_id = session(1);
    let mut journal = SessionJournal::new();
    let reader = journal.transcript_reader();
    let events = (0..READ_LIMIT + 10)
        .map(|_| AgentEvent::SessionCreated { session_id })
        .collect::<Vec<_>>();
    journal.append_events(&events);

    let first = reader.read_after(None);
    assert_eq!(first.entries().len(), READ_LIMIT);
    assert_eq!(
        first.head().map(JournalSequence::get),
        Some((READ_LIMIT + 10) as u64)
    );
    let cursor = first.entries().last().unwrap().sequence();

    let second = reader.read_after(Some(cursor));
    assert_eq!(second.entries().len(), 10);
    assert_eq!(
        second.from().map(JournalSequence::get),
        Some(READ_LIMIT as u64 + 1)
    );
}

// Reader handle을 오류 문맥에서 Debug로 출력하더라도 prompt나 기록 전체가 따라 나오면
// 관찰용 capability 자체가 민감한 Session 내용을 뜻밖의 로그로 유출할 수 있다.
#[test]
fn reader_debug_output_does_not_expose_journal_contents() {
    let session_id = session(1);
    let mut journal = SessionJournal::new();
    let reader = journal.transcript_reader();
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );

    let debug = format!("{reader:?}");

    assert_eq!(debug, "TranscriptReader { .. }");
}
