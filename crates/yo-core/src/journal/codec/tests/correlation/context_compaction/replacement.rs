use super::*;

fn checkpoint_replacement_history(strategy: ContinuationStrategy) -> Vec<JournalCommit> {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let transition = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_checkpoint_sequence(JournalSequence::new(11));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![
            semantic(
                13,
                12,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    1,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                14,
                13,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    2,
                    "managed",
                    "1.0.1",
                    identity("binding-2"),
                    identity("model"),
                    identity("session"),
                    transition,
                    strategy,
                )),
            ),
        ],
    ));
    commits
}

// checkpoint 뒤에 요청이 하나도 없으면 교체 binding은 pre-checkpoint Anchor로 돌아가지
// 않고 source_checkpoint_sequence 하나만 사용해 같은 replay root를 이어야 합니다.
#[test]
fn replacement_can_source_a_checkpoint_only_reconstruction_exclusively() {
    let commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    });
    let commits = commits
        .into_iter()
        .map(|commit| decode(&encode(&commit).unwrap()).unwrap())
        .collect::<Vec<_>>();

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.context_epoch(), Some(2));
    assert_eq!(
        recovered.context_checkpoint(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(recovered.model_replay().items().len(), 2);
}

// checkpoint에서 seed된 binding이 요청 없이 다시 교체되어도 newest executable source는
// 같은 checkpoint이며, 원 checkpoint epoch가 직전 binding epoch와 같을 필요는 없습니다.
#[test]
fn consecutive_idle_replacements_preserve_checkpoint_lineage() {
    let strategy = ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    };
    let mut commits = checkpoint_replacement_history(strategy);
    let transition = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_checkpoint_sequence(JournalSequence::new(11));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(15),
        vec![
            semantic(
                15,
                14,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    2,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    3,
                    "managed",
                    "1.0.2",
                    identity("binding-3"),
                    identity("model"),
                    identity("session"),
                    transition,
                    ContinuationStrategy::ExactReplay {
                        executor: ReplayExecutor::LocalClient,
                        replay_profile: ReplayProfile::SemanticOnly,
                    },
                )),
            ),
        ],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(3));
    assert_eq!(recovered.context_epoch(), Some(2));
    assert_eq!(
        recovered.context_checkpoint(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(recovered.model_replay().items().len(), 2);
}

// checkpoint seed에 private item이 없으면 target replay profile 변경은 손실이 아니므로
// exact transition이 새 binding의 첫 delta에서 자기 계약을 다시 선언하도록 허용합니다.
#[test]
fn replacement_allows_a_profile_change_without_private_seed_items() {
    let commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
    });

    let recovered = recover(&commits).unwrap();
    assert!(recovered.replay_contract_rebind_required());
}

// transition의 exact replay seed와 target의 이후 continuation strategy는 독립이므로
// private item이 없는 checkpoint는 backend-managed target도 안전하게 열 수 있습니다.
#[test]
fn replacement_can_seed_a_backend_managed_target_without_private_items() {
    let commits = checkpoint_replacement_history(ContinuationStrategy::BackendManagedState);

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert!(recovered.model_replay().items().is_empty());
    assert!(!recovered.replay_contract_rebind_required());
}

// retained private seed는 같은 binding identity와 replay profile일 때만 exact transition
// 가능하며, 다른 binding identity는 별도 lossy handoff 없이는 거부됩니다.
#[test]
fn replacement_private_seed_requires_the_same_binding_and_profile() {
    let private =
        ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", b"{}".to_vec())
            .unwrap();
    let retained_items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "done".to_owned(),
            refusal: None,
        },
        ModelReplayItem::ProviderPrivateAssistant { envelope: private },
    ];
    let mut base = current_history_with(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        retained_items.clone(),
    );
    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(7),
        JournalSequence::new(9),
        retained_items,
    )
    .unwrap();
    base.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![retained],
                Vec::new(),
                Vec::new(),
                JournalSequence::new(10),
            )),
        )],
    ));
    let append_replacement = |commits: &mut Vec<JournalCommit>, binding_identity| {
        let transition =
            BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
                .with_source_checkpoint_sequence(JournalSequence::new(11));
        commits.push(JournalCommit::incremental_through(
            JournalSequence::new(13),
            vec![
                semantic(
                    13,
                    12,
                    JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                        1,
                        BindingCloseReason::Replaced,
                    )),
                ),
                semantic(
                    14,
                    13,
                    JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                        2,
                        "managed",
                        "1.0.1",
                        binding_identity,
                        identity("model"),
                        identity("session"),
                        transition,
                        ContinuationStrategy::ExactReplay {
                            executor: ReplayExecutor::LocalClient,
                            replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                        },
                    )),
                ),
            ],
        ));
    };

    let mut same_binding = base.clone();
    append_replacement(&mut same_binding, identity("binding"));
    let recovered = recover(&same_binding).unwrap();
    assert!(recovered.replay_contract_rebind_required());

    let mut changed_binding = base;
    append_replacement(&mut changed_binding, identity("binding-2"));
    let error = recover(&changed_binding).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("preserve every retained private")
    );
}

// checkpoint seed를 상속한 새 binding도 첫 completed request에서 자기 replay contract를
// 정확히 한 번 다시 선언해야 하며 그 선언은 old seed contract를 안전하게 교체합니다.
#[test]
fn replacement_first_delta_establishes_its_new_replay_contract() {
    let mut commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    });
    let submission_id = super::super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(16),
        vec![
            semantic(
                15,
                14,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn: super::super::super::activity().turn(),
                            input: crate::UserInput::new("after replacement"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    2,
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
                17,
                16,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        2,
                        super::super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(15),
                        identity("request-2"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    let replay = ModelReplayDelta::new(
        Some(ModelReplayContract::new("new-system", Vec::new())),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "second".to_owned(),
            refusal: None,
        }],
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(20),
        vec![
            semantic(
                18,
                17,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn: super::super::super::activity().turn(),
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                19,
                18,
                JournalRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        2,
                        super::super::super::activity().turn_id(),
                        JournalSequence::new(16),
                        replay,
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                20,
                19,
                JournalRecord::BackendResumableOutcome(
                    BackendResumableOutcome::new(
                        2,
                        super::super::super::activity().turn_id(),
                        JournalSequence::new(16),
                        Some(identity("outcome-2")),
                        Some(JournalSequence::new(18)),
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                21,
                20,
                JournalRecord::ContinuationAnchor(
                    ContinuationAnchor::new(
                        2,
                        JournalSequence::new(16),
                        JournalSequence::new(19),
                        JournalSequence::new(19),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(
        recovered.model_replay().contract(),
        Some(&ModelReplayContract::new("new-system", Vec::new()))
    );
    assert_eq!(recovered.model_replay().items().len(), 3);

    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(17),
        JournalSequence::new(19),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "second".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let inherited_seed =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(13), JournalSequence::new(13))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(21),
        vec![semantic(
            22,
            21,
            JournalRecord::ContextCheckpoint(checkpoint_for(
                CheckpointLineage {
                    epoch: 2,
                    previous_context_epoch: 2,
                    successor_context_epoch: 3,
                    source_anchor_sequence: JournalSequence::new(20),
                    source_journal_boundary: JournalSequence::new(20),
                },
                ModelReplayContract::new("new-system", Vec::new()),
                vec![retained],
                Vec::new(),
                vec![inherited_seed],
            )),
        )],
    ));
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(3));
    assert_eq!(recovered.model_replay().items().len(), 2);
}

// replacement checkpoint 뒤 accepted request가 durable하지만 matching Anchor가 없으면
// transition source로 되돌아가 재전송하지 않고 executable continuation을 거부합니다.
#[test]
fn replacement_with_an_unanchored_request_does_not_fall_back_to_checkpoint() {
    let mut commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    });
    let submission_id = super::super::super::submission(44);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(16),
        vec![
            semantic(
                15,
                14,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn: super::super::super::activity().turn(),
                            input: crate::UserInput::new("uncertain"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    2,
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
                17,
                16,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        2,
                        super::super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(15),
                        identity("request-uncertain"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    let recovered = recover(&commits).unwrap();

    let error = crate::session_repository::build_continuation(
        recovered,
        super::super::super::activity().session_id(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("no newest durable"));
}
