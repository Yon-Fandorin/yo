use super::*;

// 새 정책·epoch·checkpoint를 물리 wire로 왕복한 뒤에도 summary와 retained group만으로
// 정확한 successor replay root와 checkpoint-only 실행 대상을 복구해야 합니다.
#[test]
fn round_trips_and_recovers_a_checkpoint_as_the_new_replay_root() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let commits = commits
        .into_iter()
        .map(|commit| decode(&encode(&commit).unwrap()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();

    assert_eq!(recovered.context_epoch(), Some(2));
    assert_eq!(
        recovered.context_checkpoint(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(recovered.continuation_anchor(), None);
    assert_eq!(recovered.model_replay().items().len(), 2);
    assert!(matches!(
        &recovered.model_replay().items()[0],
        ModelReplayItem::Message { role: ModelReplayRole::User, content, .. }
            if content == portable_body()
    ));
    let continuation = crate::session_repository::build_continuation(
        recovered,
        super::super::super::activity().session_id(),
    )
    .expect("a checkpoint without a later request is executable");
    assert_eq!(
        continuation.target().source_checkpoint_sequence(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(continuation.target().source_anchor_sequence(), None);
}

// checkpoint publish와 다음 dispatch 근거가 같은 physical append에 묶이면 checkpoint가
// 먼저 durable했다는 경계를 증명할 수 없으므로 checkpoint는 incremental commit의 끝입니다.
#[test]
fn rejects_records_after_a_checkpoint_in_the_same_incremental_commit() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(12, 11, JournalRecord::ContextCheckpoint(checkpoint())),
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("after checkpoint"),
                        },
                        super::super::super::submission(43),
                    )
                    .unwrap(),
                ),
            ),
        ],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("final record"));
}

// 첫 checkpoint root도 successor context의 durable source group으로 남아야 다음
// checkpoint가 이전 synthetic body를 loss로 선언하고 새 suffix만 retain할 수 있습니다.
#[test]
fn repeated_checkpoint_accounts_for_the_prior_checkpoint_root() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("second request"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                14,
                13,
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
                15,
                14,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(13),
                        identity("request-2"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(18),
        vec![
            semantic(
                16,
                15,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn,
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                17,
                16,
                JournalRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        1,
                        turn.turn_id(),
                        JournalSequence::new(14),
                        ModelReplayDelta::new(
                            None,
                            vec![ModelReplayItem::Message {
                                role: ModelReplayRole::Assistant,
                                content: "second answer".to_owned(),
                                refusal: None,
                            }],
                        ),
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                18,
                17,
                JournalRecord::BackendResumableOutcome(
                    BackendResumableOutcome::new(
                        1,
                        turn.turn_id(),
                        JournalSequence::new(14),
                        Some(identity("outcome-2")),
                        Some(JournalSequence::new(16)),
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                19,
                18,
                JournalRecord::ContinuationAnchor(
                    ContinuationAnchor::new(
                        1,
                        JournalSequence::new(14),
                        JournalSequence::new(17),
                        JournalSequence::new(17),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(15),
        JournalSequence::new(17),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "second answer".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let prior_root =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(11), JournalSequence::new(11))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(19),
        vec![semantic(
            20,
            19,
            JournalRecord::ContextCheckpoint(checkpoint_for(
                CheckpointLineage {
                    epoch: 1,
                    previous_context_epoch: 2,
                    successor_context_epoch: 3,
                    source_anchor_sequence: JournalSequence::new(18),
                    source_journal_boundary: JournalSequence::new(18),
                },
                ModelReplayContract::new("system", Vec::new()),
                vec![retained],
                Vec::new(),
                vec![prior_root],
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(3));
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message { content, .. }) if content == "second answer"
    ));
}

// checkpoint inline replay는 writer가 새로 꾸밀 수 있는 payload가 아니라 source
// Journal의 완료 replay group과 byte-for-byte 같아야 하므로 invented item을 거부합니다.
#[test]
fn rejects_a_checkpoint_that_invents_retained_replay() {
    let invented = ContextRetainedGroup::try_new(
        JournalSequence::new(7),
        JournalSequence::new(9),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "invented".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![invented],
                Vec::new(),
                Vec::new(),
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("exact Journal-backed replay"));
}

// retained하지 않은 완료 replay group은 visible loss로 정확히 선언해야 하므로 빈 loss
// 목록으로 source prefix를 조용히 버리는 checkpoint를 거부합니다.
#[test]
fn rejects_a_checkpoint_that_silently_omits_a_source_group() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("silently omits"));
}

// active Turn의 committed current input은 이전 Anchor 뒤 mandatory suffix이므로 source
// range와 exact user replay item을 함께 retained group에 넣은 checkpoint만 허용합니다.
#[test]
fn retains_the_complete_active_suffix_and_current_input() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("current input"),
                    },
                    super::super::super::submission(43),
                )
                .unwrap(),
            ),
        )],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(11),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "current input".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![semantic(
            13,
            12,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(11),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        }) if content == "current input"
    ));
}

// accepted request 뒤 도구 call/result Activity가 모두 닫힌 범위는 current input과 완전한
// call/output replay를 함께 inline 보존할 때만 실행 가능한 checkpoint source가 됩니다.
#[test]
fn retains_a_completed_post_tool_active_suffix() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    let call_activity = crate::ActivityRef::new(
        turn,
        crate::ActivityId::new(std::num::NonZeroU64::new(1).unwrap()),
    );
    let result_activity = crate::ActivityRef::new(
        turn,
        crate::ActivityId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(17),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("current input"),
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
                14,
                13,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(12),
                        identity("request-2"),
                    )
                    .with_context_epoch(1),
                ),
            ),
            semantic(
                15,
                14,
                JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                    activity: call_activity,
                    kind: crate::ActivityKind::ToolCall,
                }),
            ),
            SequencedJournalRecord::storage(
                ReplaySequence::new(16),
                JournalRecord::MessageEnded(MessageTerminal::new(
                    None,
                    MessageEnded::new(
                        call_activity,
                        MessageStream::ToolOutput,
                        MessageOutcome::Completed,
                        0,
                        0,
                    ),
                )),
            ),
            semantic(
                17,
                15,
                JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                    activity: call_activity,
                    outcome: crate::ActivityOutcome::Completed,
                }),
            ),
            semantic(
                18,
                16,
                JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                    activity: result_activity,
                    kind: crate::ActivityKind::ToolResult,
                }),
            ),
            SequencedJournalRecord::storage(
                ReplaySequence::new(19),
                JournalRecord::MessageEnded(MessageTerminal::new(
                    None,
                    MessageEnded::new(
                        result_activity,
                        MessageStream::ToolOutput,
                        MessageOutcome::Completed,
                        0,
                        0,
                    ),
                )),
            ),
            semantic(
                20,
                17,
                JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                    activity: result_activity,
                    outcome: crate::ActivityOutcome::Completed,
                }),
            ),
        ],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(17),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "current input".to_owned(),
                refusal: None,
            },
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "workspace contents".to_owned(),
            },
        ],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(18),
        vec![semantic(
            21,
            18,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(17),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::FunctionCallOutput { call_id, output })
            if call_id == "call-1" && output == "workspace contents"
    ));
}

// Anchor 뒤 active suffix는 단순 range와 input 포함 여부만 맞추는 payload가 아니라
// 그 한 submitted input의 exact replay 표현이어야 하므로 추가 item을 끼울 수 없습니다.
#[test]
fn rejects_an_active_suffix_that_invents_replay_items() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("current input"),
                    },
                    super::super::super::submission(43),
                )
                .unwrap(),
            ),
        )],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(11),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "current input".to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "invented".to_owned(),
                refusal: None,
            },
        ],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![semantic(
            13,
            12,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(11),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("submitted input"));
}

// accepted request는 completed Turn/delta/outcome/Anchor가 생기기 전에는 uncertain 상태이므로
// active retained range로 감싸 executable checkpoint root로 바꿀 수 없습니다.
#[test]
fn rejects_a_checkpoint_that_cuts_through_an_accepted_request() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("current input"),
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
                14,
                13,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(12),
                        identity("request-2"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(13),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "current input".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![semantic(
            15,
            14,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(13),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("submitted input"));
}

// active Anchor 뒤 semantic suffix를 loss로 돌리거나 retained range에서 빼면 현재 입력을
// 조용히 잃으므로 checkpoint recovery가 fail closed해야 합니다.
#[test]
fn rejects_a_checkpoint_that_omits_the_active_suffix() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("current input"),
                    },
                    super::super::super::submission(43),
                )
                .unwrap(),
            ),
        )],
    ));
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![semantic(
            13,
            12,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(11),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("active semantic suffix"));
}
// source boundary는 Anchor가 commit한 outcome 경계부터 checkpoint 직전 semantic
// sequence 사이의 실제 cut을 허용하므로 Anchor outcome에서 자르는 checkpoint도 복구합니다.
#[test]
fn accepts_a_checkpoint_boundary_at_the_anchor_outcome() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![
                    ContextLoss::visible_prefix_summarized(
                        JournalSequence::new(7),
                        JournalSequence::new(9),
                    )
                    .unwrap(),
                ],
                JournalSequence::new(9),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(2));
}

// context_epoch이 없던 legacy 요청·Anchor 뒤에 새 정책만 덧붙이면 두 형식의 의미를
// 안전하게 구분할 수 없으므로 current reader가 mixed graph를 거부해야 합니다.
#[test]
fn rejects_a_policy_added_after_legacy_context_records() {
    let mut commits = super::super::support::valid_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(10),
        vec![semantic(
            11,
            10,
            JournalRecord::ContextPolicyChanged(policy()),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("legacy context graph"));
}

// checkpoint가 epoch 2를 연 뒤 후속 accepted request가 superseded epoch 1을 다시 쓰면
// 과거 증거와 새 model context가 교차하므로 복구 단계에서 거부해야 합니다.
#[test]
fn rejects_a_stale_context_epoch_after_a_checkpoint() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn: super::super::super::activity().turn(),
                            input: crate::UserInput::new("more"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                14,
                13,
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
                15,
                14,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(13),
                        identity("request-2"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("current context_epoch"));
}

// warning과 trigger가 같으면 warning 관측 구간이 사라지므로 닫힌 정책 생성 자체가
// 실패해야 하며 invalid policy가 Journal wire에 도달해서는 안 됩니다.
#[test]
fn rejects_invalid_policy_bounds_before_encoding() {
    let error = ContextPolicyChanged::try_new(
        1,
        true,
        ContextStrategy::PortableSummaryV1Alpha1,
        90,
        90,
        None,
        None,
    )
    .unwrap_err();
    assert!(error.contains("warning and trigger"));
}
