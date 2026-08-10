use std::time::Duration;

use super::{
    AgentCommand, AgentEvent, JournalCommit, JournalRecord, MessageOutcome, MessageSegment,
    MessageSegmenter, MessageStream, activity, descriptor_with_path, encode, recover, sequenced,
    submission,
};

// 한 Session에서 같은 SubmissionId가 두 replay sequence에 나타나면 byte-identical
// command라도 두 번 수락된 것으로 해석하지 않고 recovery 전체를 실패시켜야 한다.
#[test]
fn recovery_rejects_a_duplicate_submission_identity_across_commits() {
    let descriptor = JournalCommit::descriptor(descriptor_with_path(b"/workspace".to_vec()));
    let command = AgentCommand::StartTurn {
        turn: activity().turn(),
        input: crate::UserInput::new("inspect"),
    };
    let first = JournalCommit::incremental(sequenced(
        2,
        [JournalRecord::CommandCommitted(
            crate::journal::CommittedCommand::submission(command.clone(), submission(9)).unwrap(),
        )],
    ));
    let duplicate = JournalCommit::incremental(sequenced(
        3,
        [JournalRecord::CommandCommitted(
            crate::journal::CommittedCommand::submission(command, submission(9)).unwrap(),
        )],
    ));

    let error = recover(&[descriptor, first, duplicate])
        .expect_err("one SubmissionId cannot identify two committed submissions");

    assert_eq!(error.commit_index(), Some(2));
    assert!(error.to_string().contains("only one committed submission"));
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
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::uncorrelated(AgentCommand::CreateSession {
                    session_id: first,
                })
                .unwrap(),
            ),
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
