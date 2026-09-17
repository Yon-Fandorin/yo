use super::*;

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
            CommittedCommand::submission(command.clone(), submission(9)).unwrap(),
        )],
    ));
    let duplicate = JournalCommit::incremental(sequenced(
        3,
        [JournalRecord::CommandCommitted(
            CommittedCommand::submission(command, submission(9)).unwrap(),
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
    let second = fixture_session(9);
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::CommandCommitted(
                CommittedCommand::uncorrelated(AgentCommand::CreateSession { session_id: first })
                    .unwrap(),
            ),
            JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: second }),
        ],
    ));

    let error = encode(&commit).expect_err("one commit cannot cross Session ownership");

    assert!(error.to_string().contains("different Sessions"));
}
