use super::*;
use crate::{TurnOutcome, journal::codec::*};

fn semantic(
    replay_sequence: u64,
    journal_sequence: u64,
    record: JournalRecord,
) -> SequencedJournalRecord {
    SequencedJournalRecord::with_journal_sequence(
        ReplaySequence::new(replay_sequence),
        JournalSequence::new(journal_sequence),
        record,
    )
}

fn identity(name: &str) -> VersionedIdentity {
    VersionedIdentity::new(format!("yo.test.{name}/v1"), format!("{name}:value"))
}

fn binding_opened() -> JournalRecord {
    JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
        1,
        "codex",
        "1.0.0",
        identity("binding"),
        identity("model"),
        identity("session"),
        BindingTransition::new(TransitionMode::Initial, CacheState::NotApplicable, None),
    ))
}

fn valid_history() -> Vec<JournalCommit> {
    let descriptor = JournalCommit::descriptor(descriptor_with_path(b"/workspace".to_vec()));
    let opened = JournalCommit::incremental_through(
        JournalSequence::new(2),
        vec![
            semantic(
                2,
                1,
                JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                    session_id: activity().session_id(),
                }),
            ),
            semantic(3, 2, binding_opened()),
        ],
    );
    let submission_id = submission(11);
    let operation_id = OperationId::from(submission_id);
    let request = JournalCommit::incremental_through(
        JournalSequence::new(5),
        vec![
            semantic(
                4,
                3,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn: activity().turn(),
                            input: crate::UserInput::new("continue"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                5,
                4,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "codex.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                6,
                5,
                JournalRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                    1,
                    activity().turn_id(),
                    operation_id,
                    JournalSequence::new(4),
                    identity("request"),
                )),
            ),
        ],
    );
    let completed = JournalCommit::incremental_through(
        JournalSequence::new(8),
        vec![
            semantic(
                7,
                6,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn: activity().turn(),
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                8,
                7,
                JournalRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                    1,
                    activity().turn_id(),
                    JournalSequence::new(5),
                    Some(identity("outcome")),
                )),
            ),
            semantic(
                9,
                8,
                JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
                    1,
                    JournalSequence::new(5),
                    JournalSequence::new(7),
                    JournalSequence::new(7),
                )),
            ),
        ],
    );
    vec![descriptor, opened, request, completed]
}

fn replacement_commit(
    epoch: u64,
    source_anchor: u64,
    first_replay: u64,
    first_journal: u64,
) -> JournalCommit {
    JournalCommit::incremental_through(
        JournalSequence::new(first_journal + 1),
        vec![
            semantic(
                first_replay,
                first_journal,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    epoch - 1,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                first_replay + 1,
                first_journal + 1,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    epoch,
                    "codex",
                    "1.0.0",
                    identity(&format!("binding-{epoch}")),
                    identity(&format!("model-{epoch}")),
                    identity(&format!("session-{epoch}")),
                    BindingTransition::new(
                        TransitionMode::ExactReplay,
                        CacheState::Lost,
                        Some(JournalSequence::new(source_anchor)),
                    ),
                )),
            ),
        ],
    )
}

fn completed_turn_in_epoch_two() -> Vec<JournalCommit> {
    let submission_id = submission(13);
    let operation_id = OperationId::from(submission_id);
    let turn = crate::TurnRef::new(
        activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(4).unwrap()),
    );
    vec![
        JournalCommit::incremental_through(
            JournalSequence::new(13),
            vec![
                semantic(
                    12,
                    11,
                    JournalRecord::CommandCommitted(
                        crate::journal::CommittedCommand::submission(
                            AgentCommand::StartTurn {
                                turn,
                                input: crate::UserInput::new("epoch two"),
                            },
                            submission_id,
                        )
                        .unwrap(),
                    ),
                ),
                semantic(
                    13,
                    12,
                    JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                        2,
                        operation_id,
                        ExchangeKind::Request,
                        ExchangeDirection::YoToBackend,
                        "codex.request/v1",
                        None,
                        None,
                        DetailAvailability::Unpersisted,
                    )),
                ),
                semantic(
                    14,
                    13,
                    JournalRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                        2,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(12),
                        identity("request-2"),
                    )),
                ),
            ],
        ),
        JournalCommit::incremental_through(
            JournalSequence::new(16),
            vec![
                semantic(
                    15,
                    14,
                    JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                        turn,
                        outcome: TurnOutcome::Completed,
                    }),
                ),
                semantic(
                    16,
                    15,
                    JournalRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                        2,
                        turn.turn_id(),
                        JournalSequence::new(13),
                        Some(identity("outcome-2")),
                    )),
                ),
                semantic(
                    17,
                    16,
                    JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
                        2,
                        JournalSequence::new(13),
                        JournalSequence::new(15),
                        JournalSequence::new(15),
                    )),
                ),
            ],
        ),
    ]
}

// 완결된 Turn의 command·exchange·accepted request·outcome·Anchor를 encode/decode한 뒤에도
// recovery가 같은 열린 epoch와 최신 Anchor를 재구성해야 안전하게 native resume할 수 있습니다.
#[test]
fn round_trips_and_recovers_a_complete_continuation_chain() {
    let commits = valid_history()
        .into_iter()
        .map(|commit| decode(&encode(&commit).unwrap()).unwrap())
        .collect::<Vec<_>>();

    let recovered = recover(&commits).expect("the complete continuation chain recovers");

    assert_eq!(recovered.binding_epoch(), Some(1));
    assert_eq!(
        recovered.continuation_anchor(),
        Some(JournalSequence::new(8))
    );
}

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

// 완결 Anchor 뒤에 새 semantic command와 outbound request가 생기면 그 suffix는 아직
// 재개 경계로 완결되지 않았으므로 이전 Anchor를 discovery 후보로 계속 내보내면 안 됩니다.
#[test]
fn clears_the_latest_anchor_when_a_new_semantic_suffix_begins() {
    let mut commits = valid_history();
    let submission_id = submission(12);
    let turn = crate::TurnRef::new(
        activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(4).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(10),
        vec![
            semantic(
                10,
                9,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("next"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                11,
                10,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    OperationId::from(submission_id),
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "codex.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
        ],
    ));

    let recovered = recover(&commits).expect("the unfinished next request remains valid history");

    assert_eq!(recovered.binding_epoch(), Some(1));
    assert_eq!(recovered.continuation_anchor(), None);
}

// revoked나 exhausted로 binding이 닫히면 과거 Anchor의 record는 replacement 검증을 위해
// 남더라도 현재 native resume 후보는 아니므로 discovery Anchor는 비워야 합니다.
#[test]
fn clears_the_latest_anchor_when_the_binding_closes() {
    let mut commits = valid_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(9),
        vec![semantic(
            10,
            9,
            JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                1,
                BindingCloseReason::Revoked,
            )),
        )],
    ));

    let recovered = recover(&commits).expect("the revoked binding is valid closed history");

    assert_eq!(recovered.binding_epoch(), None);
    assert_eq!(recovered.continuation_anchor(), None);
}

// epoch 1을 replaced로 닫은 직후 그 epoch의 최신 Anchor를 source로 쓰면 epoch 2가
// 정상적으로 열리고, 새 binding 자체가 아직 완결되지 않은 이전 Anchor 노출은 지웁니다.
#[test]
fn opens_the_next_epoch_from_the_immediately_preceding_anchor() {
    let mut commits = valid_history();
    commits.push(replacement_commit(2, 8, 10, 9));

    let recovered = recover(&commits).expect("the direct replacement lineage is valid");

    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.continuation_anchor(), None);
}

// epoch 2에도 새 Anchor가 생긴 뒤 epoch 3이 epoch 1의 오래된 Anchor로 되돌아가면 최신
// 불완결 흐름을 건너뛸 수 있으므로 바로 이전 epoch의 source가 아니라고 거부해야 합니다.
#[test]
fn rejects_a_replacement_that_falls_back_to_an_older_epoch_anchor() {
    let mut commits = valid_history();
    commits.push(replacement_commit(2, 8, 10, 9));
    commits.extend(completed_turn_in_epoch_two());
    commits.push(replacement_commit(3, 8, 18, 17));

    let error = recover(&commits).expect_err("epoch three cannot cite epoch one's stale Anchor");

    assert!(error.to_string().contains("immediately preceding epoch"));
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

// Anchor가 outcome의 JournalSequence 대신 자기 번호를 boundary로 쓰면 순환적인 완료
// 주장이 되므로 encode 단계에서 즉시 거부해야 저장소에 잘못된 재개 지점이 들어가지 않습니다.
#[test]
fn rejects_an_anchor_that_claims_its_own_sequence_as_the_boundary() {
    let mut commits = valid_history();
    let completion = commits.pop().unwrap();
    let mut records = completion.records().to_vec();
    records[2] = semantic(
        9,
        8,
        JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
            1,
            JournalSequence::new(5),
            JournalSequence::new(7),
            JournalSequence::new(8),
        )),
    );
    let invalid = JournalCommit::incremental_through(JournalSequence::new(8), records);

    let error = encode(&invalid).expect_err("an Anchor cannot claim itself as its boundary");

    assert!(error.to_string().contains("boundary"));
}
