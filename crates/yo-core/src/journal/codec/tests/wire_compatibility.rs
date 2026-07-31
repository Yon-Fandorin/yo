use super::*;

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

// 바로 앞 v1 schema에는 semantic cutoff와 message revision 필드가 없었다. decoder는
// 그 형식만 명시적으로 revision 1과 마지막 record cutoff로 복원해 기존 log를 읽어야 한다.
#[test]
fn decodes_the_supported_v1_message_shape_with_explicit_legacy_defaults() {
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);
    segmenter.push_text("legacy", Duration::ZERO);
    let commit =
        JournalCommit::incremental(sequenced(1, [segmenter.finish(MessageOutcome::Completed)]));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["schema"] = serde_json::Value::String("yo.semantic-journal-commit/v1".to_owned());
    wire.as_object_mut().unwrap().remove("journal_cutoff");
    let terminal = wire["records"][0].as_object_mut().unwrap();
    terminal["final_segment"]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    terminal["ended"]
        .as_object_mut()
        .unwrap()
        .remove("revision");

    let decoded = decode(&serde_json::to_string(&wire).unwrap()).unwrap();
    let JournalRecord::MessageEnded(terminal) = decoded.records()[0].record() else {
        panic!("the legacy terminal remains a typed message record");
    };

    assert_eq!(decoded.semantic_cutoff().get(), 1);
    assert_eq!(terminal.final_segment().unwrap().revision(), 1);
    assert_eq!(terminal.ended().revision(), 1);
}

// v1에서는 ActivityStarted와 ActivityFinished가 message lifecycle record가 아니었다.
// text가 없던 기존 activity도 새 v2 규칙의 MessageEnded를 요구하지 않고 복구해야 한다.
#[test]
fn decodes_a_v1_empty_activity_with_its_original_recovery_rules() {
    let activity = activity();
    let empty_terminal =
        MessageSegmenter::new(activity, MessageStream::Agent).finish(MessageOutcome::Completed);
    let commit = JournalCommit::snapshot(sequenced(
        1,
        [
            JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::AgentMessage,
            }),
            empty_terminal,
            JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            }),
        ],
    ));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["schema"] = serde_json::Value::String("yo.semantic-journal-commit/v1".to_owned());
    wire.as_object_mut().unwrap().remove("journal_cutoff");
    wire["records"].as_array_mut().unwrap().remove(1);

    let decoded = decode(&serde_json::to_string(&wire).unwrap()).unwrap();
    let recovered = recover(&[decoded]).unwrap();

    assert_eq!(recovered.records().len(), 2);
    assert!(recovered.recovery_commit().is_none());
}

// v2가 revision을 생략했는데도 legacy 기본값을 적용하면 손상된 최신 record를 과거
// schema로 오인한다. legacy 보정은 schema가 정확히 v1일 때만 허용해야 한다.
#[test]
fn rejects_a_v2_message_that_omits_its_revision() {
    let commit =
        JournalCommit::incremental(sequenced(
            1,
            [MessageSegmenter::new(activity(), MessageStream::Agent)
                .finish(MessageOutcome::Completed)],
        ));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["records"][0]["ended"]
        .as_object_mut()
        .unwrap()
        .remove("revision");

    let error = decode(&serde_json::to_string(&wire).unwrap())
        .expect_err("the current schema requires an explicit revision");

    assert!(error.to_string().contains("revision is required"));
}
