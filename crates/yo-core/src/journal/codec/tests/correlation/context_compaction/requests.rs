use super::*;

// 한 Turn의 첫 provider request가 이미 durable하게 수락된 뒤 도구 결과로 이어지는
// successor request는 SubmissionId가 없으므로 writer가 정한 deterministic identity로
// 복구되어야 한다.
#[test]
fn accepts_a_writer_assigned_internal_successor_request() {
    let mut commits = current_history();
    commits.pop();
    let exchange_sequence = JournalSequence::new(7);
    let operation_id = OperationId::for_internal_request(
        super::super::super::activity().session_id(),
        super::super::super::activity().turn_id(),
        exchange_sequence,
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(8),
        vec![
            semantic(
                8,
                7,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                9,
                8,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::super::activity().turn_id(),
                        operation_id,
                        exchange_sequence,
                        identity("successor-request"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));

    recover(&commits).unwrap();
}

// 동일한 deterministic identity라도 완료된 Turn에 사후 요청을 붙일 수는 없다.
#[test]
fn rejects_a_writer_assigned_request_after_turn_completion() {
    let mut commits = current_history();
    let exchange_sequence = JournalSequence::new(11);
    let operation_id = OperationId::for_internal_request(
        super::super::super::activity().session_id(),
        super::super::super::activity().turn_id(),
        exchange_sequence,
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(
                12,
                11,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                13,
                12,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::super::activity().turn_id(),
                        operation_id,
                        exchange_sequence,
                        identity("late-request"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("writer-assigned successor"));
}

// 새 Turn의 첫 request 전에 압축이 끝나는 round-zero 경로는 checkpoint가 command를
// durable하게 가로지른 경우에만 writer-assigned 첫 request를 허용한다.
#[test]
fn accepts_the_first_writer_assigned_request_after_an_active_checkpoint() {
    let mut commits = current_history();
    let second_turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
    let submission_id = super::super::super::submission(43);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn: second_turn,
                            input: crate::UserInput::new("round zero"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                13,
                12,
                JournalRecord::EventCommitted(AgentEvent::TurnStarted { turn: second_turn }),
            ),
        ],
    ));
    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(11),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "round zero".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![semantic(
            14,
            13,
            JournalRecord::ContextCheckpoint(checkpoint_for(
                CheckpointLineage {
                    epoch: 1,
                    previous_context_epoch: 1,
                    successor_context_epoch: 2,
                    source_anchor_sequence: JournalSequence::new(10),
                    source_journal_boundary: JournalSequence::new(11),
                },
                ModelReplayContract::new("system", Vec::new()),
                vec![retained],
                Vec::new(),
                vec![
                    ContextLoss::visible_prefix_summarized(
                        JournalSequence::new(7),
                        JournalSequence::new(9),
                    )
                    .unwrap(),
                ],
            )),
        )],
    ));
    let exchange_sequence = JournalSequence::new(14);
    let operation_id = OperationId::for_internal_request(
        second_turn.session_id(),
        second_turn.turn_id(),
        exchange_sequence,
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(15),
        vec![
            semantic(
                15,
                14,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        second_turn.turn_id(),
                        operation_id,
                        exchange_sequence,
                        identity("round-zero-request"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(2));
}
