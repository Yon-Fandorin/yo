use std::{num::NonZeroU64, time::Duration};

use super::{
    JournalCommit, JournalRecord, MessageOutcome, MessageSegment, MessageSegmenter, MessageStream,
    SequencedJournalRecord, decode, encode, recover,
};
use crate::{
    ActivityId, ActivityRef, AgentCommand, AgentEvent, JournalSequence, SessionId, TurnId, TurnRef,
    UserInput,
};

fn activity() -> ActivityRef {
    let session_id = SessionId::new(NonZeroU64::new(1).unwrap());
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(3).unwrap()))
}

fn sequenced(
    first: u64,
    records: impl IntoIterator<Item = JournalRecord>,
) -> Vec<SequencedJournalRecord> {
    records
        .into_iter()
        .enumerate()
        .map(|(offset, record)| {
            SequencedJournalRecord::new(
                JournalSequence::new(first + u64::try_from(offset).unwrap()),
                record,
            )
        })
        .collect()
}

// 16KiB 경계에 걸친 다중 바이트 UTF-8 입력을 agent message로 나누어도 각 segment가
// 유효한 UTF-8 bound 안에 있고 다시 이으면 원문과 바이트 단위로 같아야 한다.
#[test]
fn splits_agent_text_at_valid_utf8_boundaries_without_changing_content() {
    let text = format!("{}한끝", "a".repeat((16 * 1024) - 1));
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);

    let mut segments = segmenter.push_text(&text, Duration::ZERO);
    segments.extend(segmenter.flush_boundary());

    assert_eq!(
        segments
            .iter()
            .map(MessageSegment::text)
            .collect::<String>(),
        text
    );
    assert!(
        segments
            .iter()
            .all(|segment| segment.text().len() <= 16 * 1024)
    );
    assert_eq!(segments.len(), 2);
}

// 같은 크기의 text라도 tool output은 64KiB까지 한 segment로 유지되어 agent message와
// 서로 다른 계약 bound가 실제로 구분되는지 검증한다.
#[test]
fn uses_the_larger_bound_for_tool_output() {
    let text = "x".repeat(32 * 1024);
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::ToolOutput);

    assert!(segmenter.push_text(&text, Duration::ZERO).is_empty());
    let segment = segmenter
        .flush_boundary()
        .expect("a non-text boundary forces the buffered tool output");

    assert_eq!(segment.text(), text);
}

// 가장 오래된 미커밋 바이트가 정확히 1초에 도달하면 크기가 작아도 강제로 segment를
// 만들어 crash 시 무한정 큰 volatile tail이 남지 않게 해야 한다.
#[test]
fn flushes_a_small_tail_when_it_reaches_one_second() {
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);
    segmenter.push_text("tail", Duration::from_millis(10));

    assert!(segmenter.flush_due(Duration::from_millis(1009)).is_none());
    assert_eq!(
        segmenter
            .flush_due(Duration::from_millis(1010))
            .expect("the oldest byte reached one second")
            .text(),
        "tail"
    );
}

// message 종료 시 남은 tail과 MessageEnded가 한 record batch에 함께 나오고 seal의
// segment 수·UTF-8 byte 수가 전체 원문을 정확히 설명해야 불완전 복원을 탐지할 수 있다.
#[test]
fn seals_the_final_tail_with_exact_reconstruction_counts() {
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);
    segmenter.push_text("안녕", Duration::ZERO);

    let record = segmenter.finish(MessageOutcome::Completed);

    let JournalRecord::MessageEnded(terminal) = &record else {
        panic!("termination must produce one atomic terminal record");
    };
    let final_segment = terminal
        .final_segment()
        .expect("the non-empty final tail is embedded with its seal");
    let ended = terminal.ended();
    assert_eq!(final_segment.text(), "안녕");
    assert_eq!(ended.segment_count(), 1);
    assert_eq!(ended.utf8_bytes(), "안녕".len() as u64);
    assert_eq!(ended.outcome(), &MessageOutcome::Completed);
}

// command와 그 결과 event를 한 semantic commit으로 codec 왕복해도 record 순서와
// 독립 JournalSequence가 그대로 남아야 한 physical append가 원자적 원인·결과가 된다.
#[test]
fn round_trips_one_atomic_command_commit() {
    let session_id = activity().session_id();
    let turn = activity().turn();
    let commit = JournalCommit::incremental(sequenced(
        7,
        [
            JournalRecord::CommandCommitted(AgentCommand::StartTurn {
                turn,
                input: UserInput::new("검사"),
            }),
            JournalRecord::EventCommitted(AgentEvent::TurnStarted { turn }),
        ],
    ));

    let decoded = decode(&encode(&commit).expect("commit encodes")).expect("commit decodes");

    assert_eq!(decoded, commit);
    assert_eq!(decoded.journal_cutoff().map(JournalSequence::get), Some(8));
    assert_eq!(session_id, turn.session_id());
}

// complete snapshot이 sequence 1이 아닌 중간 suffix에서 시작하면 저장 후 recovery gate를
// 거짓으로 해제할 수 있으므로 codec admission 단계에서 physical append 전에 거부해야 한다.
#[test]
fn rejects_a_snapshot_that_does_not_begin_with_the_complete_journal() {
    let commit = JournalCommit::snapshot(sequenced(
        5,
        [JournalRecord::EventCommitted(AgentEvent::SessionCreated {
            session_id: activity().session_id(),
        })],
    ));

    let error = encode(&commit).expect_err("an incomplete snapshot is not durable");

    assert!(error.to_string().contains("begin at sequence 1"));
}

// 한 semantic commit에 서로 다른 Session identity가 섞이면 physical envelope의 Session을
// 어느 쪽으로도 정직하게 표현할 수 없으므로 codec이 mixed authority를 거부해야 한다.
#[test]
fn rejects_records_from_different_sessions_in_one_commit() {
    let first = activity().session_id();
    let second = SessionId::new(NonZeroU64::new(9).unwrap());
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::CommandCommitted(AgentCommand::CreateSession { session_id: first }),
            JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: second }),
        ],
    ));

    let error = encode(&commit).expect_err("one commit cannot cross Session ownership");

    assert!(error.to_string().contains("different Sessions"));
}

// backend TextDelta를 일반 AgentEvent로 durable codec에 넣으면 transport chunk가 replay
// authority가 되므로 bounded MessageSegment 경계를 우회하는 text event를 거부해야 한다.
#[test]
fn rejects_raw_text_updates_that_bypass_message_segments() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity: activity(),
            update: crate::ActivityUpdate::TextDelta("raw delta".to_owned()),
        })],
    ));

    let error = encode(&commit).expect_err("raw backend text is not durable semantics");

    assert!(error.to_string().contains("bounded MessageSegments"));
}

// 완결 seal의 segment 수가 durable prefix와 다르면 손상된 message를 completed로
// 재생하지 않고 복구 오류로 거부해야 한다.
#[test]
fn rejects_a_terminal_seal_that_does_not_match_durable_segments() {
    let activity = activity();
    let mut segmenter = MessageSegmenter::new(activity, MessageStream::Agent);
    let segment = segmenter
        .push_text(&"x".repeat(16 * 1024), Duration::ZERO)
        .pop()
        .expect("the size boundary emits a segment");
    let mismatched = super::MessageEnded::new(
        activity,
        MessageStream::Agent,
        MessageOutcome::Completed,
        2,
        16 * 1024,
    );
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::MessageSegment(segment),
            JournalRecord::MessageEnded(super::MessageTerminal::new(None, mismatched)),
        ],
    ));

    let error = recover(&[commit]).expect_err("the inconsistent seal is corruption");

    assert!(error.to_string().contains("does not match"));
}

// crash로 마지막 MessageEnded만 없는 durable message를 복구하면 다음 sequence에
// interrupted seal을 제안해 partial임을 보존하고 completed로 추정하지 않아야 한다.
#[test]
fn recovery_seals_an_unterminated_message_as_interrupted() {
    let activity = activity();
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity,
            MessageStream::Agent,
            1,
            "partial".to_owned(),
        ))],
    ));

    let recovered = recover(&[commit]).expect("the durable prefix recovers");
    let seal = recovered
        .recovery_commit()
        .expect("an unterminated message requires a recovery seal");
    let JournalRecord::MessageEnded(terminal) = seal.records()[0].record() else {
        panic!("recovery emits a terminal seal");
    };
    let ended = terminal.ended();

    assert_eq!(seal.records()[0].sequence().get(), 2);
    assert_eq!(ended.outcome(), &MessageOutcome::Interrupted);
    assert_eq!(ended.segment_count(), 1);
    assert_eq!(ended.utf8_bytes(), 7);
    assert_eq!(recovered.records().len(), 1);
    let snapshot = recovered.complete_snapshot();
    assert_eq!(snapshot.records().len(), 2);
    assert_eq!(snapshot.records()[0].sequence().get(), 1);
    assert_eq!(snapshot.records()[1].sequence().get(), 2);
}

// 열린 durable message 뒤의 commit에 later event가 이미 기록되어 있으면 끝에서 seal을
// 덧붙여 순서를 바꾸지 말고, event보다 먼저 interrupted seal할 수 없던 기록으로 거부해야 한다.
#[test]
fn recovery_rejects_a_later_event_after_an_unterminated_message() {
    let activity = activity();
    let message = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity,
            MessageStream::Agent,
            1,
            "partial".to_owned(),
        ))],
    ));
    let later_event = JournalCommit::incremental(sequenced(
        2,
        [JournalRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                activity,
                outcome: crate::ActivityOutcome::Completed,
            },
        )],
    ));

    let error =
        recover(&[message, later_event]).expect_err("the later event crossed an open message");

    assert!(error.to_string().contains("sealed before"));
    assert_eq!(error.commit_index(), Some(1));
}

// sequence 1부터 시작해도 terminal seal이 없는 snapshot은 복구 시 interrupted record를
// 합성해야 하므로 complete authority가 아니며 codec admission에서 저장 전에 거부해야 한다.
#[test]
fn rejects_a_snapshot_that_requires_an_interrupted_recovery_seal() {
    let snapshot = JournalCommit::snapshot(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity(),
            MessageStream::Agent,
            1,
            "partial".to_owned(),
        ))],
    ));

    let error = encode(&snapshot).expect_err("an open message makes the snapshot incomplete");

    assert!(error.to_string().contains("recovery repair"));
}

// 같은 message에 두 번째 MessageEnded가 나타나면 각 seal이 0-byte 완료처럼 보이더라도
// terminal identity가 모호해지므로 recovery가 중복 종료를 손상으로 거부해야 한다.
#[test]
fn recovery_rejects_a_duplicate_terminal_seal() {
    let ended = super::MessageEnded::new(
        activity(),
        MessageStream::Agent,
        MessageOutcome::Completed,
        0,
        0,
    );
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::MessageEnded(super::MessageTerminal::new(None, ended.clone())),
            JournalRecord::MessageEnded(super::MessageTerminal::new(None, ended)),
        ],
    ));

    let error = recover(&[commit]).expect_err("a duplicate terminal seal is corruption");

    assert!(error.to_string().contains("more than one"));
}

// codec가 선언된 stream bound보다 큰 직접 생성 segment를 허용하면 buffer를 우회한
// 손상 레코드가 durable storage에 들어갈 수 있으므로 encode 단계에서 거부해야 한다.
#[test]
fn rejects_an_oversized_segment_even_when_the_segmenter_is_bypassed() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity(),
            MessageStream::Agent,
            1,
            "x".repeat((16 * 1024) + 1),
        ))],
    ));

    let error = encode(&commit).expect_err("admission enforces the semantic segment bound");

    assert!(error.to_string().contains("exceeds"));
}
