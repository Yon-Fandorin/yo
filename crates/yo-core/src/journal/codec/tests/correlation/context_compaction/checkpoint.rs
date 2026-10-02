#[cfg(test)]
use std::num::NonZeroU64;

use super::*;
#[cfg(test)]
use crate::journal::CommittedCommand;
#[cfg(test)]
use crate::session_repository;

#[cfg(test)]
fn checkpoint_with_corrections(
    source_corrections: [&str; 2],
    retained_corrections: &[&str],
) -> Vec<JournalCommit> {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("current input"),
                        },
                        super::super::super::submission(43),
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn,
                            input: crate::UserInput::new(source_corrections[0]),
                        },
                        super::super::super::submission(44),
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                14,
                13,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn,
                            input: crate::UserInput::new(source_corrections[1]),
                        },
                        super::super::super::submission(45),
                    )
                    .unwrap(),
                ),
            ),
        ],
    ));
    let mut items = vec![ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "current input".to_owned(),
        refusal: None,
    }];
    items.extend(
        retained_corrections
            .iter()
            .map(|content| ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: (*content).to_owned(),
                refusal: None,
            }),
    );
    let active =
        ContextRetainedGroup::try_new(JournalSequence::new(11), JournalSequence::new(13), items)
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
    commits
}

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
    let continuation = session_repository::build_continuation(
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
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(12, 11, JournalRecord::ContextCheckpoint(checkpoint())),
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
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
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
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
                    CommittedCommand::submission(
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
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
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

// 같은 Turn의 여러 조향 제출은 내용이 같아도 별도 제출로 보존하고 Journal 순서를 따라야 합니다.
#[test]
fn retains_ordered_active_steering_provenance_on_recovery() {
    let commits = checkpoint_with_corrections(
        ["first correction", "second correction"],
        &["first correction", "second correction"],
    );

    let recovered = recover(&commits).unwrap();
    let user_items = recovered
        .model_replay()
        .items()
        .iter()
        .filter(|item| {
            matches!(
                item,
                ModelReplayItem::Message {
                    role: ModelReplayRole::User,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        user_items.as_slice(),
        [
            ModelReplayItem::Message {
                content: summary,
                ..
            },
            ModelReplayItem::Message { content: first, .. },
            ModelReplayItem::Message { content: second, .. },
            ModelReplayItem::Message { content: third, .. }
        ] if summary == portable_body()
            && first == "current input"
            && second == "first correction"
            && third == "second correction"
    ));
}

// Recovery는 조향 입력을 요약하거나 다시 꾸미지 않고 누락·중복·순서 변경을 거부해야 합니다.
#[test]
fn rejects_missing_duplicated_or_reordered_active_corrections() {
    for retained in [
        vec!["first correction"],
        vec!["first correction", "second correction", "second correction"],
        vec!["second correction", "first correction"],
    ] {
        let commits =
            checkpoint_with_corrections(["first correction", "second correction"], &retained);
        assert!(recover(&commits).is_err());
    }
}

// 같은 문장을 두 번 제출한 사실은 텍스트 중복 제거로 하나로 합쳐서는 안 됩니다.
#[test]
fn retains_identical_text_as_two_active_submissions() {
    let commits = checkpoint_with_corrections(["repeat", "repeat"], &["repeat", "repeat"]);

    let recovered = recover(&commits).unwrap();
    let repeats = recovered
        .model_replay()
        .items()
        .iter()
        .filter(|item| {
            matches!(
                item,
                ModelReplayItem::Message {
                    role: ModelReplayRole::User,
                    content,
                    ..
                } if content == "repeat"
            )
        })
        .count();
    assert_eq!(repeats, 2);
}

// accepted request 뒤 도구 call/result Activity가 모두 닫힌 범위는 current input과 완전한
// call/output replay를 함께 inline 보존할 때만 실행 가능한 checkpoint source가 됩니다.
fn completed_post_tool_active_suffix_checkpoint(
    correction_after_response: bool,
    with_nonclosing_record_after_correction: bool,
    tool_result_failed: bool,
) -> Vec<JournalCommit> {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    let call_activity =
        crate::ActivityRef::new(turn, crate::ActivityId::new(NonZeroU64::new(1).unwrap()));
    let result_activity =
        crate::ActivityRef::new(turn, crate::ActivityId::new(NonZeroU64::new(2).unwrap()));
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    let tool_failure = crate::Failure::new("tool returned an error");
    let tool_output = if tool_result_failed {
        "tool returned an error"
    } else {
        "workspace contents"
    };
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(17),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
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
                        if tool_result_failed {
                            MessageOutcome::Failed(tool_failure.clone())
                        } else {
                            MessageOutcome::Completed
                        },
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
                    outcome: if tool_result_failed {
                        crate::ActivityOutcome::Failed(tool_failure)
                    } else {
                        crate::ActivityOutcome::Completed
                    },
                }),
            ),
        ],
    ));
    if correction_after_response {
        let mut records = vec![semantic(
            21,
            18,
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
                    AgentCommand::SteerTurn {
                        turn,
                        input: crate::UserInput::new("correction"),
                    },
                    super::super::super::submission(44),
                )
                .unwrap(),
            ),
        )];
        if with_nonclosing_record_after_correction {
            records.push(semantic(
                22,
                19,
                JournalRecord::CommandCommitted(
                    CommittedCommand::uncorrelated(AgentCommand::CompactContext { guidance: None })
                        .unwrap(),
                ),
            ));
        }
        commits.push(JournalCommit::incremental_through(
            JournalSequence::new(if with_nonclosing_record_after_correction {
                19
            } else {
                18
            }),
            records,
        ));
    }
    let mut retained_items = vec![
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
            output: tool_output.to_owned(),
        },
    ];
    if correction_after_response {
        retained_items.push(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "correction".to_owned(),
            refusal: None,
        });
    }
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(if with_nonclosing_record_after_correction {
            19
        } else if correction_after_response {
            18
        } else {
            17
        }),
        retained_items,
    )
    .unwrap();
    let source_boundary = active.last_sequence();
    let checkpoint_sequence = source_boundary.advance_by(1);
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        checkpoint_sequence,
        vec![semantic(
            source_boundary.advance_by(4).get(),
            checkpoint_sequence.get(),
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                source_boundary,
            )),
        )],
    ));

    commits
}

// 완료된 도구 call/output과 assistant를 포함한 active suffix는 correction 없이도 정확히 복구되어야
// 합니다.
#[test]
fn retains_a_completed_post_tool_active_suffix() {
    let commits = completed_post_tool_active_suffix_checkpoint(false, false, false);
    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::FunctionCallOutput { call_id, output })
            if call_id == "call-1" && output == "workspace contents"
    ));
}

// 완료된 tool 결과 뒤에 도착한 correction은 ActivityFinished 뒤 source boundary에
// 별도 입력으로 보존됩니다.
#[test]
fn retains_a_late_correction_after_a_completed_tool_group() {
    let commits = completed_post_tool_active_suffix_checkpoint(true, false, false);

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        }) if content == "correction"
    ));
}

// 뒤늦은 correction 뒤 non-closing command까지 source range에 넣으면 input이 실제 group 끝과
// 일치하지 않으므로 recovery가 malformed checkpoint를 거부해야 합니다.
#[test]
fn rejects_a_post_tool_active_suffix_ending_after_its_last_correction() {
    let commits = completed_post_tool_active_suffix_checkpoint(true, true, false);

    let error = recover(&commits).unwrap_err();
    assert!(
        error.to_string().contains(
            "context retained active group is not the exact Journal-backed submitted input"
        )
    );
}

// 정상적으로 반환된 도구 오류도 정확한 call/output과 실패 종료 Activity가 있으면
// 완결된 replay 경계이므로 checkpoint로 보존할 수 있습니다.
#[test]
fn retains_a_failed_tool_output_after_its_closed_activity() {
    let commits = completed_post_tool_active_suffix_checkpoint(false, false, true);

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::FunctionCallOutput { call_id, output })
            if call_id == "call-1" && output == "tool returned an error"
    ));
}

// Anchor 뒤 active suffix는 단순 range와 input 포함 여부만 맞추는 payload가 아니라
// 그 한 submitted input의 exact replay 표현이어야 하므로 추가 item을 끼울 수 없습니다.
#[test]
fn rejects_an_active_suffix_that_invents_replay_items() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
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

// accepted request 뒤 완료된 AgentMessage나 ToolCall 근거가 오기 전에는 제출 목록 또는
// 꾸며낸 assistant item만으로 uncertain request를 checkpoint root로 바꿀 수 없습니다.
#[test]
fn rejects_a_checkpoint_that_cuts_through_an_accepted_request() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
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
                    CommittedCommand::submission(
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
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![semantic(
            15,
            14,
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
                    AgentCommand::SteerTurn {
                        turn,
                        input: crate::UserInput::new("correction"),
                    },
                    super::super::super::submission(44),
                )
                .unwrap(),
            ),
        )],
    ));

    let active_items = [
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "current input".to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "correction".to_owned(),
                refusal: None,
            },
        ],
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "current input".to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "invented before response".to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "correction".to_owned(),
                refusal: None,
            },
        ],
    ];
    for items in active_items {
        let mut candidate = commits.clone();
        let active = ContextRetainedGroup::try_new(
            JournalSequence::new(11),
            JournalSequence::new(14),
            items,
        )
        .unwrap();
        let old_prefix = ContextLoss::visible_prefix_summarized(
            JournalSequence::new(7),
            JournalSequence::new(9),
        )
        .unwrap();
        candidate.push(JournalCommit::incremental_through(
            JournalSequence::new(15),
            vec![semantic(
                16,
                15,
                JournalRecord::ContextCheckpoint(checkpoint_with(
                    ModelReplayContract::new("system", Vec::new()),
                    vec![active],
                    Vec::new(),
                    vec![old_prefix],
                    JournalSequence::new(14),
                )),
            )],
        ));

        let error = recover(&candidate).unwrap_err();
        assert!(error.to_string().contains("submitted input"));
    }
}

// response가 열린 동안 durable된 correction은 completed assistant와 함께 정확한
// ActivityFinished boundary까지 active group으로 복구할 수 있습니다.
fn completed_assistant_active_suffix_checkpoint(
    provider_private: bool,
    response_interrupted: bool,
    reasoning_tail: bool,
) -> Vec<JournalCommit> {
    let private = ProviderPrivateReplayEnvelope::new(
        crate::provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
        br#"{"content":"answer","reasoning_content":"kept"}"#.to_vec(),
    )
    .unwrap();
    let mut commits = if provider_private {
        current_history_with(
            ReplayProfile::ProviderPrivateLocalPlaintext,
            vec![
                ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: "prior answer".to_owned(),
                    refusal: None,
                },
                ModelReplayItem::ProviderPrivateAssistant {
                    envelope: private.clone(),
                },
            ],
        )
    } else {
        current_history()
    };
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    let assistant_activity =
        crate::ActivityRef::new(turn, crate::ActivityId::new(NonZeroU64::new(1).unwrap()));
    let reasoning_activity =
        crate::ActivityRef::new(turn, crate::ActivityId::new(NonZeroU64::new(2).unwrap()));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
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
    let mut response_records = vec![
        semantic(
            15,
            14,
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
                    AgentCommand::SteerTurn {
                        turn,
                        input: crate::UserInput::new("correction"),
                    },
                    super::super::super::submission(44),
                )
                .unwrap(),
            ),
        ),
        semantic(
            16,
            15,
            JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity: assistant_activity,
                kind: crate::ActivityKind::AgentMessage,
            }),
        ),
        SequencedJournalRecord::storage(
            ReplaySequence::new(17),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::new(
                    assistant_activity,
                    MessageStream::Agent,
                    if response_interrupted {
                        MessageOutcome::Interrupted
                    } else {
                        MessageOutcome::Completed
                    },
                    0,
                    0,
                ),
            )),
        ),
        semantic(
            18,
            16,
            JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity: assistant_activity,
                outcome: if response_interrupted {
                    crate::ActivityOutcome::Interrupted
                } else {
                    crate::ActivityOutcome::Completed
                },
            }),
        ),
    ];
    if reasoning_tail {
        response_records.extend([
            semantic(
                19,
                17,
                JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                    activity: reasoning_activity,
                    kind: crate::ActivityKind::ModelWork,
                }),
            ),
            SequencedJournalRecord::storage(
                ReplaySequence::new(20),
                JournalRecord::MessageEnded(MessageTerminal::new(
                    None,
                    MessageEnded::new(
                        reasoning_activity,
                        MessageStream::for_activity(crate::ActivityKind::ModelWork),
                        MessageOutcome::Completed,
                        0,
                        0,
                    ),
                )),
            ),
            semantic(
                21,
                18,
                JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                    activity: reasoning_activity,
                    outcome: crate::ActivityOutcome::Completed,
                }),
            ),
        ]);
    }
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(if reasoning_tail { 18 } else { 16 }),
        response_records,
    ));
    let mut active_items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "current input".to_owned(),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "answer".to_owned(),
            refusal: None,
        },
    ];
    if provider_private {
        active_items.push(ModelReplayItem::ProviderPrivateAssistant {
            envelope: private.clone(),
        });
    }
    active_items.push(ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "correction".to_owned(),
        refusal: None,
    });
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(if reasoning_tail { 18 } else { 16 }),
        active_items,
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    let mut losses = vec![old_prefix];
    if provider_private {
        losses.push(
            ContextLoss::provider_private_dropped(
                private.schema(),
                u64::try_from(private.payload().len()).unwrap(),
                JournalSequence::new(8),
            )
            .unwrap(),
        );
    }
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(if reasoning_tail { 19 } else { 17 }),
        vec![semantic(
            if reasoning_tail { 22 } else { 19 },
            if reasoning_tail { 19 } else { 17 },
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                losses,
                JournalSequence::new(if reasoning_tail { 18 } else { 16 }),
            )),
        )],
    ));

    commits
}

// 응답 streaming 중 durable된 correction도 completed assistant와 함께 active suffix로 보존됩니다.
#[test]
fn retains_a_completed_assistant_with_a_correction_admitted_during_streaming() {
    let commits = completed_assistant_active_suffix_checkpoint(false, false, false);

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        }) if content == "correction"
    ));
}

// provider-private replay도 visible assistant와 exact envelope 순서를 유지하며
// correction을 포함한 같은 completed boundary까지 복구됩니다.
#[test]
fn retains_provider_private_assistant_with_a_streaming_correction() {
    let commits = completed_assistant_active_suffix_checkpoint(true, false, false);

    let recovered = recover(&commits).unwrap();
    assert!(
        recovered
            .model_replay()
            .items()
            .iter()
            .any(|item| { matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }) })
    );
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        }) if content == "correction"
    ));
}

// visible assistant item이 있어도 interrupted Activity는 완료된 model response
// 경계가 아니므로 recovery source로 허용하지 않습니다.
#[test]
fn rejects_an_interrupted_assistant_active_suffix() {
    let commits = completed_assistant_active_suffix_checkpoint(false, true, false);

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("submitted input"));
}

// completed Assistant 뒤에 닫힌 reasoning ModelWork가 와도 response와 correction provenance를
// 복구합니다.
#[test]
fn retains_a_completed_assistant_with_a_closed_reasoning_tail() {
    let commits = completed_assistant_active_suffix_checkpoint(false, false, true);

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        }) if content == "correction"
    ));
}

// active Anchor 뒤 semantic suffix를 loss로 돌리거나 retained range에서 빼면 현재 입력을
// 조용히 잃으므로 checkpoint recovery가 fail closed해야 합니다.
#[test]
fn rejects_a_checkpoint_that_omits_the_active_suffix() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::super::activity().session_id(),
        crate::TurnId::new(NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
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
                    CommittedCommand::submission(
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
