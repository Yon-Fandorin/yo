use super::*;

fn rewrite_session_ids_as_legacy(value: &mut serde_json::Value, legacy_id: u64) {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                if name == "session_id" {
                    *value = serde_json::Value::from(legacy_id);
                } else {
                    rewrite_session_ids_as_legacy(value, legacy_id);
                }
            }
        },
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_session_ids_as_legacy(value, legacy_id);
            }
        },
        _ => {},
    }
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
    rewrite_session_ids_as_legacy(&mut wire, 1);
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
    rewrite_session_ids_as_legacy(&mut wire, 1);
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
    wire["schema"] = serde_json::Value::String("yo.semantic-journal-commit/v2".to_owned());
    rewrite_session_ids_as_legacy(&mut wire, 1);
    wire["records"][0]["ended"]
        .as_object_mut()
        .unwrap()
        .remove("revision");

    let error = decode(&serde_json::to_string(&wire).unwrap())
        .expect_err("the current schema requires an explicit revision");

    assert!(error.to_string().contains("revision is required"));
}

// 유효한 v2 semantic commit의 모든 중첩 Session identity가 숫자 형식이어도 기존
// cutoff와 record를 보존해 읽어야 실제 과거 Journal 조회 호환성을 증명할 수 있다.
#[test]
fn decodes_a_valid_v2_commit_with_nested_numeric_session_identities() {
    let commit = JournalCommit::incremental(sequenced(
        4,
        [
            JournalRecord::CommandCommitted(AgentCommand::StartTurn {
                turn: activity().turn(),
                input: UserInput::new("legacy v2"),
            }),
            JournalRecord::EventCommitted(AgentEvent::TurnStarted {
                turn: activity().turn(),
            }),
        ],
    ));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["schema"] = serde_json::Value::String("yo.semantic-journal-commit/v2".to_owned());
    rewrite_session_ids_as_legacy(&mut wire, 7);

    let decoded = decode(&serde_json::to_string(&wire).unwrap()).expect("valid v2 commit decodes");

    assert_eq!(decoded.records().len(), 2);
    assert_eq!(decoded.semantic_cutoff().get(), 5);
    assert_eq!(decoded.session_id().unwrap().to_string(), "legacy:7");
}

// 이전 schema 숫자 identity를 복구한 commit은 조회에는 쓸 수 있지만 v3으로 다시
// encode하면 새 UUIDv7 Session처럼 보이므로 쓰기 직전에 명시적으로 거부해야 한다.
#[test]
fn refuses_to_encode_a_recovered_legacy_session() {
    let legacy_session = crate::SessionId::from_legacy(NonZeroU64::MIN);
    let turn = TurnRef::new(legacy_session, TurnId::new(NonZeroU64::MIN));
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::CommandCommitted(AgentCommand::StartTurn {
            turn,
            input: UserInput::new("legacy"),
        })],
    ));

    let error = encode(&commit).expect_err("legacy Session commits are read-only");

    assert!(error.to_string().contains("read-only"));
}

// v3 payload가 cutoff를 선언해도 record가 비어 있으면 Session ID를 찾는 과정에서
// panic하지 않고 손상된 semantic commit이라는 typed 오류를 반환해야 한다.
#[test]
fn rejects_an_empty_v3_commit_without_panicking() {
    let payload = serde_json::json!({
        "schema": "yo.semantic-journal-commit/v3",
        "kind": "incremental",
        "journal_cutoff": 1,
        "first_sequence": 1,
        "records": []
    });

    let error = decode(&payload.to_string()).expect_err("empty commits are invalid");

    assert!(error.to_string().contains("at least one Journal record"));
}
