use std::{num::NonZeroU64, time::Duration};

use super::{
    JournalCommit, JournalRecord, MessageOutcome, MessageSegment, MessageSegmenter, MessageStream,
    ReplaySequence, SequencedJournalRecord, decode, encode, recover,
};
use crate::{
    ActivityId, ActivityRef, AgentCommand, AgentEvent, HostWorkspacePath, JournalSequence,
    SessionDescriptor, TurnId, TurnRef,
};

mod wire_compatibility;

fn activity() -> ActivityRef {
    let session_id = crate::fixture_session(1);
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

fn expect_segment(record: JournalRecord) -> MessageSegment {
    let JournalRecord::MessageSegment(segment) = record else {
        panic!("the boundary must emit a text segment");
    };
    segment
}

fn descriptor_with_path(path: Vec<u8>) -> SessionDescriptor {
    SessionDescriptor::for_session(
        activity().session_id(),
        "10000000-0000-4000-8000-000000000001"
            .parse()
            .expect("the test Host fixture is a UUIDv4"),
        HostWorkspacePath::from_unix_bytes(path)
            .expect("the test workspace path is absolute and NUL-free"),
    )
}

// descriptor-only 첫 commit은 semantic JournalSequence를 만들지 않으면서도 UUIDv7 Session,
// Host, 정규화 경로와 시작 시각을 v1 payload에서 손실 없이 왕복해야 한다.
#[test]
fn round_trips_the_initial_descriptor_without_a_semantic_cutoff() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let commit = JournalCommit::descriptor(descriptor.clone());

    let encoded = encode(&commit).expect("the descriptor commit encodes");
    let decoded = decode(&encoded).expect("the descriptor commit decodes");
    let recovered = recover(std::slice::from_ref(&decoded)).expect("the descriptor recovers");

    assert_eq!(decoded.journal_cutoff(), None);
    assert_eq!(recovered.descriptor(), Some(&descriptor));
    assert_eq!(recovered.journal_cutoff(), None);
}

// Unix workspace가 UTF-8이 아니어도 lossy 문자열로 바꾸지 않고 명시적인 unix_bytes
// 표현을 사용해야 다른 Host가 로컬 path 규칙을 적용하지 않고 원본 바이트를 보존한다.
#[test]
fn preserves_a_non_utf8_workspace_path_with_an_explicit_wire_encoding() {
    let descriptor = descriptor_with_path(vec![b'/', b'w', 0xff]);
    let encoded = encode(&JournalCommit::descriptor(descriptor.clone())).unwrap();

    assert!(encoded.contains("\"encoding\":\"unix_bytes\""));
    let decoded = decode(&encoded).unwrap();
    let JournalRecord::SessionDescriptor(decoded_descriptor) = decoded.records()[0].record() else {
        panic!("the first record remains a Session descriptor");
    };
    assert_eq!(decoded_descriptor, &descriptor);
}

// remote Host의 path를 reader filesystem에서 다시 resolve하면 안 되지만 `.`·`..`, 빈
// component, trailing separator처럼 canonicalize 결과가 만들지 않는 lexical alias는
// 같은 workspace를 여러 값으로 나누므로 손상된 descriptor로 거부해야 한다.
#[test]
fn rejects_lexically_noncanonical_workspace_paths_from_the_wire() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let encoded = encode(&JournalCommit::descriptor(descriptor)).unwrap();

    for path in [
        "/workspace/../other",
        "/workspace/.",
        "/workspace//child",
        "/workspace/",
    ] {
        let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
        wire["records"][0]["descriptor"]["workspace_path"]["value"] =
            serde_json::Value::String(path.to_owned());

        let error = decode(&wire.to_string()).expect_err("lexical aliases are not canonical");

        assert!(error.to_string().contains("host-normalized"));
    }
}

// descriptor의 명시적 시작 시각이 UUIDv7 내부 millisecond와 다르면 두 개의 시작점을
// 만들게 되므로 semantic JSON decoder가 모순된 descriptor를 거부해야 한다.
#[test]
fn rejects_a_descriptor_start_time_that_disagrees_with_its_session_id() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let encoded = encode(&JournalCommit::descriptor(descriptor)).unwrap();
    let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
    wire["records"][0]["descriptor"]["start_time_unix_millis"] = serde_json::Value::from(1_u64);

    let error = decode(&wire.to_string()).expect_err("mismatched start times are corrupt");

    assert!(error.to_string().contains("does not match"));
}

// descriptor가 ReplaySequence 1이 아닌 곳에 나타나면 session prefix가 두 개로 갈라질 수
// 있으므로 codec이 physical append 전에 잘못 놓인 descriptor를 거부해야 한다.
#[test]
fn rejects_a_descriptor_outside_replay_sequence_one() {
    let commit = JournalCommit::incremental_through(
        JournalSequence::new(1),
        vec![SequencedJournalRecord::new(
            ReplaySequence::new(2),
            JournalRecord::SessionDescriptor(descriptor_with_path(b"/workspace".to_vec())),
        )],
    );

    let error = encode(&commit).expect_err("a misplaced descriptor is not durable");

    assert!(error.to_string().contains("replay-sequence-one"));
}

// 16KiB 경계에 걸친 다중 바이트 UTF-8 입력을 agent message로 나누어도 각 segment가
// 유효한 UTF-8 bound 안에 있고 다시 이으면 원문과 바이트 단위로 같아야 한다.
#[test]
fn splits_agent_text_at_valid_utf8_boundaries_without_changing_content() {
    let text = format!("{}한끝", "a".repeat((16 * 1024) - 1));
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);

    let mut segments = segmenter.push_text(&text, Duration::ZERO);
    if let Some(record) = segmenter.flush_boundary() {
        segments.push(expect_segment(record));
    }

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
    let segment = expect_segment(
        segmenter
            .flush_boundary()
            .expect("a non-text boundary forces the buffered tool output"),
    );

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
        expect_segment(
            segmenter
                .flush_due(Duration::from_millis(1010))
                .expect("the oldest byte reached one second"),
        )
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
    let second = crate::fixture_session(9);
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

// non-text 경계에서 pending text가 segment로 먼저 강제 저장되면 message가 아직 끝나지
// 않았더라도 다른 Activity 사건을 기록할 수 있어야 한다. 재시작 시에는 마지막 durable
// 위치에서 열린 message만 interrupted로 봉인해 동시 Activity의 원래 순서를 보존한다.
#[test]
fn recovery_preserves_an_event_after_a_forced_message_segment() {
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
        [JournalRecord::EventCommitted(AgentEvent::TurnFinished {
            turn: activity.turn(),
            outcome: crate::TurnOutcome::Interrupted,
        })],
    ));

    let recovered = recover(&[message, later_event]).expect("the ordering boundary is replayable");

    assert_eq!(recovered.records().len(), 2);
    assert!(recovered.recovery_commit().is_some());
}

// ActivityStarted 뒤 첫 text segment가 저장되기 전에 crash가 나도 durable activity 자체가
// 열린 zero-byte message를 증명한다. 복구는 이를 completed로 꾸미지 않고 interrupted
// MessageEnded(0 segments, 0 bytes)로 봉인해야 한다.
#[test]
fn recovery_seals_a_started_message_before_its_first_text() {
    let activity = activity();
    let started = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity,
            kind: crate::ActivityKind::AgentMessage,
        })],
    ));

    let recovered = recover(&[started]).expect("the zero-byte live message is recoverable");
    let seal = recovered
        .recovery_commit()
        .expect("the crash leaves one interrupted seal");
    let JournalRecord::MessageEnded(terminal) = seal.records()[0].record() else {
        panic!("recovery emits a typed message terminal");
    };
    assert_eq!(terminal.ended().segment_count(), 0);
    assert_eq!(terminal.ended().utf8_bytes(), 0);
    assert_eq!(terminal.ended().outcome(), &MessageOutcome::Interrupted);
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
