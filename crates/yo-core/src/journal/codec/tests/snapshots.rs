use std::time::Duration;

use super::{
    AgentEvent, JournalCommit, JournalRecord, JournalSequence, MessageOutcome, MessageSegment,
    MessageSegmenter, MessageStream, ReplaySequence, SequencedJournalRecord, activity,
    descriptor_with_path, encode, recover, sequenced,
};

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

// 뒤 snapshot이 sequence 1부터 다시 시작하더라도 이미 읽은 descriptor를 다른 값으로
// 바꾸면 과거 semantic prefix 재작성에 해당하므로 recovery가 교체를 거부해야 합니다.
#[test]
fn recovery_rejects_a_snapshot_that_rewrites_the_existing_prefix() {
    let descriptor = JournalCommit::descriptor(descriptor_with_path(b"/workspace".to_vec()));
    let rewritten = JournalCommit::snapshot_through(
        JournalSequence::new(1),
        vec![
            SequencedJournalRecord::storage(
                ReplaySequence::new(1),
                JournalRecord::SessionDescriptor(descriptor_with_path(b"/other".to_vec())),
            ),
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(2),
                JournalSequence::new(1),
                JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                    session_id: activity().session_id(),
                }),
            ),
        ],
    );

    let error = recover(&[descriptor, rewritten])
        .expect_err("a later snapshot cannot replace the recovered descriptor");

    assert!(
        error
            .to_string()
            .contains("preserve the recovered semantic prefix")
    );
}

// 이미 durable segment가 나온 뒤 backend가 권위 있는 TextSnapshot으로 내용을 교체해도
// 새 revision이 index를 다시 시작하고 terminal seal이 그 최종 revision만 검증해야 한다.
// 그래야 이전 전송 조각은 진단 이력으로 남기면서 재생 결과를 낡은 prefix와 섞지 않는다.
#[test]
fn replacement_revision_supersedes_an_earlier_segment_without_mutating_it() {
    let activity = activity();
    let mut segmenter = MessageSegmenter::new(activity, MessageStream::Agent);
    let first = segmenter
        .push_text(&"a".repeat(16 * 1024), Duration::ZERO)
        .pop()
        .expect("the first revision reaches its size boundary");
    let replacement = segmenter.replace_text("authoritative", Duration::from_millis(1));
    assert!(
        replacement.is_empty(),
        "the short replacement remains buffered"
    );
    let terminal = segmenter.finish(MessageOutcome::Completed);
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(first), terminal],
    ));

    let recovered = recover(&[commit]).expect("the replacement revision is replayable");
    let JournalRecord::MessageSegment(first) = recovered.records()[0].record() else {
        panic!("the original immutable segment remains visible");
    };
    let JournalRecord::MessageEnded(terminal) = recovered.records()[1].record() else {
        panic!("the replacement is sealed atomically");
    };
    let final_segment = terminal
        .final_segment()
        .expect("the terminal carries the short replacement tail");

    assert_eq!(first.revision(), 1);
    assert_eq!(final_segment.revision(), 2);
    assert_eq!(final_segment.index(), 1);
    assert_eq!(final_segment.text(), "authoritative");
    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.ended().segment_count(), 1);
    assert_eq!(terminal.ended().utf8_bytes(), 13);
}

// 권위 있는 empty snapshot은 이전 revision을 빈 본문으로 교체한다. text segment가 없어도
// 다음 revision의 zero-byte terminal이 그 교체를 완전히 표현하고 복구에 성공해야 한다.
#[test]
fn empty_replacement_snapshot_is_a_recoverable_zero_byte_revision() {
    let activity = activity();
    let mut segmenter = MessageSegmenter::new(activity, MessageStream::Agent);
    assert!(segmenter.replace_text("", Duration::ZERO).is_empty());
    let terminal = segmenter.finish(MessageOutcome::Completed);
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: crate::ActivityKind::AgentMessage,
            }),
            terminal,
            JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity,
                outcome: crate::ActivityOutcome::Completed,
            }),
        ],
    ));

    let recovered = recover(&[commit]).expect("the empty replacement is a complete revision");
    let JournalRecord::MessageEnded(terminal) = recovered.records()[1].record() else {
        panic!("the empty replacement has a typed terminal");
    };

    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.ended().segment_count(), 0);
    assert_eq!(terminal.ended().utf8_bytes(), 0);
    assert!(recovered.recovery_commit().is_none());
}

// 아직 segment가 나오지 않은 replacement snapshot들은 같은 unpublished revision을
// 덮어쓴다. 빈 snapshot 뒤 최종 text가 와도 revision gap 없이 하나의 revision 2가 된다.
#[test]
fn consecutive_unpublished_snapshots_share_the_next_revision() {
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);
    segmenter.replace_text("", Duration::ZERO);
    segmenter.replace_text("final", Duration::from_millis(1));
    let JournalRecord::MessageEnded(terminal) = segmenter.finish(MessageOutcome::Completed) else {
        panic!("the final replacement has a typed terminal");
    };

    assert_eq!(terminal.final_segment().unwrap().revision(), 2);
    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.final_segment().unwrap().text(), "final");
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
