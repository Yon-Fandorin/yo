use super::{
    super::{
        JournalCommit, JournalRecord, MessageSegment, MessageStream, ReplaySequence,
        SequencedJournalRecord, activity, decode, descriptor_with_path, encode, recover,
        submission,
    },
    support::{semantic, valid_history},
};
use crate::{
    AgentEvent, JournalSequence,
    journal::codec::{
        BackendExchangeObserved, DetailAvailability, ExchangeDirection, ExchangeKind, OperationId,
    },
};

// semantic record는 명시적인 JournalSequence를 가지지만 message segment는 저장 경계일
// 뿐이므로 같은 wire records 배열에서도 journal_sequence field가 없어야 합니다.
#[test]
fn writes_journal_sequence_only_for_semantic_records() {
    let commit = JournalCommit::incremental_through(
        JournalSequence::new(2),
        vec![
            semantic(
                1,
                2,
                JournalRecord::EventCommitted(AgentEvent::TurnStarted {
                    turn: activity().turn(),
                }),
            ),
            SequencedJournalRecord::new(
                ReplaySequence::new(2),
                JournalRecord::MessageSegment(MessageSegment::new(
                    activity(),
                    MessageStream::Agent,
                    1,
                    "hello".to_owned(),
                )),
            ),
        ],
    );

    let wire: serde_json::Value = serde_json::from_str(&encode(&commit).unwrap()).unwrap();

    assert_eq!(wire["records"][0]["journal_sequence"], 2);
    assert!(wire["records"][1].get("journal_sequence").is_none());
}

// command/event/correlation처럼 의미 순서를 이루는 record에서 journal_sequence가 빠지면
// replay 순번으로 추측하지 않고 decoder가 필수 field 누락으로 거부해야 합니다.
#[test]
fn rejects_a_semantic_record_without_journal_sequence() {
    let commit = JournalCommit::incremental_through(
        JournalSequence::new(1),
        vec![semantic(
            1,
            1,
            JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                session_id: activity().session_id(),
            }),
        )],
    );
    let mut wire: serde_json::Value = serde_json::from_str(&encode(&commit).unwrap()).unwrap();
    wire["records"][0]
        .as_object_mut()
        .unwrap()
        .remove("journal_sequence");

    let error = decode(&wire.to_string()).expect_err("semantic sequence is mandatory");

    assert!(error.to_string().contains("journal_sequence"), "{error}");
}

// message segment는 physical replay 순서만 가지는 저장 record이므로 journal_sequence를
// 덧붙인 wire를 허용하면 안 되고, 닫힌 schema가 알 수 없는 field로 거부해야 합니다.
#[test]
fn rejects_journal_sequence_on_a_storage_only_record() {
    let commit = JournalCommit::incremental(vec![SequencedJournalRecord::new(
        ReplaySequence::new(1),
        JournalRecord::MessageSegment(MessageSegment::new(
            activity(),
            MessageStream::Agent,
            1,
            "hello".to_owned(),
        )),
    )]);
    let mut wire: serde_json::Value = serde_json::from_str(&encode(&commit).unwrap()).unwrap();
    wire["records"][0]["journal_sequence"] = 1.into();

    let error = decode(&wire.to_string()).expect_err("storage records reject semantic sequence");

    assert!(error.to_string().contains("unknown field"), "{error}");
}

// UUID parser가 읽을 수 있더라도 대문자나 하이픈 없는 operation_id는 같은 UUID의 다른
// byte 표현이므로 exchange와 accepted-request record 모두 decoder가 거부해야 합니다.
#[test]
fn rejects_noncanonical_operation_identity_text() {
    let request = valid_history().remove(2);
    let encoded = encode(&request).unwrap();

    for record_index in [1, 2] {
        let mut wire: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        let canonical = wire["records"][record_index]["operation_id"]
            .as_str()
            .unwrap();
        wire["records"][record_index]["operation_id"] =
            canonical.replace('-', "").to_uppercase().into();

        let error = decode(&wire.to_string()).expect_err("operation IDs have one wire spelling");

        assert!(error.to_string().contains("canonical lowercase"), "{error}");
    }
}

// 앞 commit의 cutoff가 5라면 다음 incremental record가 2를 새 사건처럼 재사용해서는
// 안 되며, recovery가 이미 durable한 의미 prefix 안으로 들어오는 번호를 거부해야 합니다.
#[test]
fn rejects_an_incremental_sequence_inside_the_preceding_cutoff() {
    let descriptor = JournalCommit::descriptor(descriptor_with_path(b"/workspace".to_vec()));
    let first = JournalCommit::incremental_through(
        JournalSequence::new(5),
        vec![semantic(
            2,
            1,
            JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                session_id: activity().session_id(),
            }),
        )],
    );
    let reused = JournalCommit::incremental_through(
        JournalSequence::new(5),
        vec![semantic(
            3,
            2,
            JournalRecord::EventCommitted(AgentEvent::TurnStarted {
                turn: activity().turn(),
            }),
        )],
    );

    let error = recover(&[descriptor, first, reused])
        .expect_err("incremental sequences cannot enter the durable prefix");

    assert!(error.to_string().contains("preceding journal_cutoff"));
}

// response가 같은 방향의 request를 가리키면 operation ID가 같더라도 요청과 응답의
// 방향 계약을 위반하므로 correlation graph admission이 실패해야 합니다.
#[test]
fn rejects_a_response_with_the_same_direction_as_its_request() {
    let mut commits = valid_history();
    commits.truncate(2);
    let operation_id = OperationId::from(submission(21));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(4),
        vec![
            semantic(
                4,
                3,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "test.request/v1",
                    None,
                    None,
                    DetailAvailability::Missing,
                )),
            ),
            semantic(
                5,
                4,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Response,
                    ExchangeDirection::YoToBackend,
                    "test.response/v1",
                    Some(JournalSequence::new(3)),
                    None,
                    DetailAvailability::Missing,
                )),
            ),
        ],
    ));

    let error = recover(&commits).expect_err("same-direction responses are invalid");

    assert!(error.to_string().contains("opposite-direction"));
}
