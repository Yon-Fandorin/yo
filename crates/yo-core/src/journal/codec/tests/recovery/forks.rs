use super::*;

// 부모 epoch 3의 완전한 private replay를 child epoch 1에 등록하고 snapshot 재시작에도 원본 bytes를
// 보존합니다.
#[test]
fn initial_fork_bootstrap_round_trips_exact_and_private_replay_without_synthetic_requests() {
    for private in [false, true] {
        let records = fork_records(private);
        let JournalRecord::InitialForkSeed(seed) = &records[2] else {
            panic!("seed fixture")
        };
        let ForkSeed::ExactReplay(exact) = seed.seed() else {
            panic!("exact fixture")
        };
        let expected = exact.clone();
        let encoded = encode(&fork_commit(records)).unwrap();
        let recovered = recover(&[decode(&encoded).unwrap()]).unwrap();
        assert_eq!(recovered.binding_epoch(), Some(1));
        assert_eq!(recovered.context_epoch(), Some(1));
        assert_eq!(recovered.initial_fork_seed(), Some(JournalSequence::new(2)));
        assert_eq!(recovered.continuation_anchor(), None);
        assert_eq!(recovered.context_checkpoint(), None);
        assert_eq!(recovered.model_replay().items(), expected.items());
        assert_eq!(
            recovered.model_replay().contract(),
            Some(expected.contract())
        );
        assert!(recovered.records().iter().all(|entry| !matches!(
            entry.record(),
            JournalRecord::BackendRequestAccepted(_)
                | JournalRecord::ContinuationAnchor(_)
                | JournalRecord::ModelReplayDelta(_)
        )));
        let snapshot = recovered.complete_snapshot();
        let reloaded = recover(&[decode(&encode(&snapshot).unwrap()).unwrap()]).unwrap();
        assert_eq!(reloaded.model_replay(), recovered.model_replay());
        assert_eq!(reloaded.initial_fork_seed(), Some(JournalSequence::new(2)));
        let persisted = reloaded
            .records()
            .iter()
            .find_map(|entry| match entry.record() {
                JournalRecord::InitialForkSeed(seed) => Some(seed),
                _ => None,
            })
            .unwrap();
        let ForkSeed::ExactReplay(exact) = persisted.seed() else {
            panic!("persisted exact seed")
        };
        assert_eq!(exact, &expected);
        assert_eq!(exact.item_origins()[0].original().binding_epoch(), 3);
        assert_eq!(exact.item_origins()[0].original().context_epoch(), 2);
    }
}

// bootstrap policy 누락, 분리 publish, 중복 seed는 실행 가능한 빈 Session으로 복구하면 안 됩니다.
#[test]
fn initial_fork_bootstrap_rejects_missing_policy_split_publish_and_duplicate_seed() {
    let records = fork_records(false);
    let mut missing_policy = records.clone();
    missing_policy.pop();
    assert!(recover(&[fork_commit(missing_policy)]).is_err());
    let complete = fork_commit(records.clone());
    let descriptor = JournalCommit::descriptor(fixture_descriptor(fixture_session(82)));
    let rest = JournalCommit::incremental_through(
        JournalSequence::new(4),
        complete.records()[1..].to_vec(),
    );
    assert!(recover(&[descriptor, rest]).is_err());
    let mut duplicate = records.clone();
    duplicate.insert(3, records[2].clone());
    assert!(recover(&[fork_commit(duplicate)]).is_err());
    let first = fork_commit(records.clone());
    let late = JournalCommit::incremental_through(
        JournalSequence::new(5),
        vec![SequencedJournalRecord::with_journal_sequence(
            ReplaySequence::new(6),
            JournalSequence::new(5),
            records[2].clone(),
        )],
    );
    assert!(recover(&[first, late]).is_err());
}

// 최초 binding이 source와 다른 계정·model을 명명하거나 seed를 잘못 참조하면 bootstrap을 거부합니다.
#[test]
fn initial_fork_bootstrap_rejects_binding_model_and_seed_reference_mismatch() {
    for (binding_identity, model, seed_sequence) in [
        ("wrong-account", "model", 2),
        ("account", "wrong-model", 2),
        ("account", "model", 1),
    ] {
        let mut records = fork_records(true);
        let source = fork_binding(ReplayProfile::ProviderPrivateLocalPlaintext);
        records[3] = JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
            1,
            source.backend_kind(),
            source.backend_version(),
            VersionedIdentity::new("binding/v1", binding_identity),
            VersionedIdentity::new("model/v1", model),
            VersionedIdentity::new("locator/v1", "child"),
            BindingTransition::initial_fork(JournalSequence::new(seed_sequence)),
            source.continuation_strategy(),
        ));
        assert!(recover(&[fork_commit(records)]).is_err());
    }
    let mut same_parent = serde_json::from_str::<serde_json::Value>(
        &encode(&fork_commit(fork_records(false))).unwrap(),
    )
    .unwrap();
    same_parent["records"][2]["parent_session_id"] =
        serde_json::Value::String(fixture_session(82).to_string());
    assert!(decode(&same_parent.to_string()).is_err());
}

// empty fork는 replay·Anchor를 만들지 않으며 backend-native proof의 알 수 없는 schema는 fail
// closed입니다.
#[test]
fn empty_fork_recovers_without_replay_and_unknown_native_proof_is_rejected() {
    let mut empty = fork_records(false);
    empty[2] = JournalRecord::InitialForkSeed(Box::new(
        InitialForkSeed::new(
            fixture_session(82),
            fixture_session(81),
            ForkSource::Empty,
            ForkSeed::Empty,
            vec![],
            2,
        )
        .unwrap(),
    ));
    let recovered = recover(&[fork_commit(empty)]).unwrap();
    assert!(recovered.model_replay().items().is_empty());
    assert_eq!(recovered.initial_fork_seed(), None);
    assert_eq!(recovered.continuation_anchor(), None);
    let parent = BackendBindingEvidence::new(
        "native",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "parent"),
        ContinuationStrategy::BackendManagedState,
    );
    let candidate = BackendBindingEvidence::new(
        "native",
        "1",
        parent.binding_identity().clone(),
        parent.model_identity().clone(),
        BackendIdentity::new("locator/v1", "child"),
        ContinuationStrategy::BackendManagedState,
    );
    let mut native = fork_records(false);
    native[2] = JournalRecord::InitialForkSeed(Box::new(
        InitialForkSeed::new(
            fixture_session(82),
            fixture_session(81),
            ForkSource::Anchor(
                ForkSourcePoint::new(
                    3,
                    2,
                    JournalSequence::new(9),
                    JournalSequence::new(8),
                    parent,
                )
                .unwrap(),
            ),
            ForkSeed::BackendNative {
                source_boundary_evidence: VersionedIdentity::new(
                    "unsupported-boundary/v1",
                    "proof",
                ),
                candidate_binding: candidate.clone(),
            },
            vec![],
            2,
        )
        .unwrap(),
    ));
    native[3] = JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
        1,
        "native",
        "1",
        VersionedIdentity::new("binding/v1", "account"),
        VersionedIdentity::new("model/v1", "model"),
        VersionedIdentity::new("locator/v1", "child"),
        BindingTransition::initial_fork(JournalSequence::new(2)),
        candidate.continuation_strategy(),
    ));
    native.pop();
    let error = recover(&[fork_commit(native)]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("native fork source boundary schema has no supported verifier")
    );
}

// 다른 child로 먼저 검증된 seed도 실제 descriptor의 child가 parent/original이면 다시 거부해야
// 합니다.
#[test]
fn fork_seed_revalidates_the_actual_descriptor_child() {
    let records = fork_records(true);
    let JournalRecord::InitialForkSeed(seed) = &records[2] else {
        panic!("fork fixture")
    };
    assert!(seed.validate_child(fixture_session(82)).is_ok());
    assert!(seed.validate_child(fixture_session(81)).is_err());
    let mut mismatched = records;
    mismatched[0] = JournalRecord::SessionDescriptor(fixture_descriptor(fixture_session(81)));
    mismatched[1] = JournalRecord::EventCommitted(AgentEvent::SessionCreated {
        session_id: fixture_session(81),
    });
    assert!(recover(&[fork_commit(mismatched)]).is_err());
}

// child가 첫 요청을 수락한 뒤 완료하지 않았다면 seed hint로 이전 context를 자동 재전송하면 안
// 됩니다.
#[test]
fn accepted_child_suffix_disables_initial_seed_continuation() {
    let initial = fork_commit(fork_records(true));
    let (_, request) = fork_child_request();
    let commits = [initial, request]
        .iter()
        .map(|commit| decode(&encode(commit).unwrap()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.initial_fork_seed(), None);
    assert_eq!(recovered.continuation_anchor(), None);
    assert!(session_repository::build_continuation(recovered, fixture_session(82)).is_err());
}

// 새 child delta만 baseline 뒤에 붙이고 원본 epoch 3와 새 private epoch 1을 snapshot에서 구별해
// 보존합니다.
#[test]
fn completed_child_turn_appends_once_after_imported_baseline() {
    for private in [false, true] {
        let initial_records = fork_records(private);
        let JournalRecord::InitialForkSeed(seed) = &initial_records[2] else {
            panic!("initial seed")
        };
        let ForkSeed::ExactReplay(baseline) = seed.seed() else {
            panic!("exact seed")
        };
        let baseline = baseline.clone();
        let (turn, request) = fork_child_request();
        let mut suffix = vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "child followup".into(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "new child answer".into(),
                refusal: None,
            },
        ];
        if private {
            suffix.push(ModelReplayItem::ProviderPrivateAssistant {
                envelope: ProviderPrivateReplayEnvelope::new(
                    provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
                    br#"{"reasoning_content":"new private bytes","content":"new child answer"}"#
                        .to_vec(),
                )
                .unwrap(),
            });
        }
        let complete = fork_child_commit(
            8,
            vec![
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn,
                    outcome: TurnOutcome::Completed,
                }),
                JournalRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        1,
                        turn.turn_id(),
                        JournalSequence::new(7),
                        ModelReplayDelta::new(None, suffix.clone()),
                    )
                    .with_context_epoch(1),
                ),
                JournalRecord::BackendResumableOutcome(
                    BackendResumableOutcome::new(
                        1,
                        turn.turn_id(),
                        JournalSequence::new(7),
                        Some(VersionedIdentity::new("outcome/v1", "child-completed")),
                        Some(JournalSequence::new(9)),
                    )
                    .with_context_epoch(1),
                ),
                JournalRecord::ContinuationAnchor(
                    ContinuationAnchor::new(
                        1,
                        JournalSequence::new(7),
                        JournalSequence::new(10),
                        JournalSequence::new(10),
                    )
                    .with_context_epoch(1),
                ),
            ],
        );
        let commits = [fork_commit(initial_records), request, complete]
            .iter()
            .map(|commit| decode(&encode(commit).unwrap()).unwrap())
            .collect::<Vec<_>>();
        let recovered = recover(&commits).unwrap();
        let expected = baseline
            .items()
            .iter()
            .chain(&suffix)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(recovered.initial_fork_seed(), None);
        assert_eq!(
            recovered.continuation_anchor(),
            Some(JournalSequence::new(11))
        );
        assert_eq!(recovered.model_replay().items(), expected);
        assert_eq!(
            recovered.model_replay().contract(),
            Some(baseline.contract())
        );
        let encoded = encode(&recovered.complete_snapshot()).unwrap();
        let reloaded = recover(&[decode(&encoded).unwrap()]).unwrap();
        assert_eq!(reloaded.model_replay().items(), expected);
        assert_eq!(reloaded.model_replay_groups().len(), 2);
        let suffix_delta = reloaded
            .records()
            .iter()
            .find_map(|record| match record.record() {
                JournalRecord::ModelReplayDelta(delta) => Some(delta),
                _ => None,
            })
            .unwrap();
        assert_eq!(suffix_delta.epoch(), 1);
        assert_eq!(suffix_delta.context_epoch(), Some(1));
        let persisted_seed = reloaded
            .records()
            .iter()
            .find_map(|record| match record.record() {
                JournalRecord::InitialForkSeed(seed) => Some(seed),
                _ => None,
            })
            .unwrap();
        let ForkSeed::ExactReplay(exact) = persisted_seed.seed() else {
            panic!("retained seed")
        };
        assert_eq!(exact, &baseline);
        assert_eq!(exact.item_origins()[0].original().binding_epoch(), 3);
        assert!(session_repository::build_continuation(reloaded, fixture_session(82)).is_ok());
    }
}

// 부모 파일 없이 child만 디스크에 저장·재개하고 inherited usage를 child 사용량으로 합산하지
// 않습니다.
#[test]
fn fork_repository_reopens_private_seed_without_ancestor_files_or_inherited_usage() {
    let directory = ForkRepositoryDirectory(env::temp_dir().join(format!(
        "yo-fork-independent-{}",
        crate::SessionId::new().unwrap()
    )));
    fs::create_dir(&directory.0).unwrap();
    let child = fixture_session(82);
    let parent = fixture_session(81);
    let mut records = fork_records(true);
    let JournalRecord::InitialForkSeed(original) = &records[2] else {
        panic!("seed fixture")
    };
    let source_activity = ActivityRef::new(
        TurnRef::new(parent, TurnId::new(1.try_into().unwrap())),
        ActivityId::new(1.try_into().unwrap()),
    );
    let receipt = serde_json::json!({"schema":"yo.model-usage-receipt/v1", "response_id":"parent-response", "round":1,
        "provider":"managed", "account":"account", "model":"model", "connector":"connector", "api_dialect":"dialect",
        "base_url":"https://managed.invalid", "usage":{"input_tokens":17,"output_tokens":2,"total_tokens":19,"reasoning_tokens":null},
        "cache_read_input_tokens":{"availability":"absent","source_profile":"managed.cache-read/v1"}}).to_string();
    let source_events = vec![
        AgentEvent::ActivityStarted {
            activity: source_activity,
            kind: ActivityKind::ModelWork,
        },
        AgentEvent::ActivityUpdated {
            activity: source_activity,
            update: ActivityUpdate::TextSnapshot(receipt),
        },
        AgentEvent::ActivityFinished {
            activity: source_activity,
            outcome: ActivityOutcome::Completed,
        },
    ];
    let source_projection = source_events
        .iter()
        .cloned()
        .map(crate::TranscriptRecord::EventCommitted)
        .collect::<Vec<_>>();
    assert_eq!(
        SessionUsageProjection::from_records(&source_projection)
            .unwrap()
            .receipts()
            .len(),
        1
    );
    let history = source_events
        .into_iter()
        .enumerate()
        .map(|(index, event)| {
            let sequence = JournalSequence::new(index as u64 + 2);
            ForkHistoryEntry::new(
                parent,
                ForkHistoryCoordinate::Journal { sequence },
                SequencedJournalRecord::with_journal_sequence(
                    ReplaySequence::new(sequence.get()),
                    sequence,
                    JournalRecord::EventCommitted(event),
                ),
            )
            .unwrap()
        })
        .collect();
    let seed = InitialForkSeed::new(
        child,
        parent,
        original.source().clone(),
        original.seed().clone(),
        history,
        4096,
    )
    .unwrap();
    let ForkSeed::ExactReplay(expected) = seed.seed() else {
        panic!("private seed")
    };
    let expected = expected.clone();
    records[2] = JournalRecord::InitialForkSeed(Box::new(seed));
    let commit = fork_commit(records);
    {
        let repository = LocalSessionRepository::open(&directory.0, 4 * 1024 * 1024).unwrap();
        let mut journal = JournalRepository::new(repository);
        journal.append(child, &commit).unwrap();
    }
    assert!(!directory.0.join(format!("{parent}.jsonl")).exists());
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    assert_eq!(reader.discover().unwrap().len(), 1);
    let history = read_stored_session(&reader, child).unwrap();
    assert!(history.discovery_consistent());
    assert!(history.session_usage().unwrap().receipts().is_empty());
    let entries = reader.read_after(child, None, 4).unwrap();
    assert_eq!(entries.len(), 1);
    let commits = entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.initial_fork_seed(), Some(JournalSequence::new(2)));
    assert_eq!(recovered.model_replay().items(), expected.items());
    assert_eq!(
        recovered.model_replay().contract(),
        Some(expected.contract())
    );
    let persisted = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            JournalRecord::InitialForkSeed(seed) => Some(seed),
            _ => None,
        })
        .unwrap();
    assert_eq!(persisted.history().len(), 3);
    assert!(
        persisted
            .history()
            .iter()
            .all(|entry| entry.source_session_id() == parent)
    );
    let ForkSeed::ExactReplay(exact) = persisted.seed() else {
        panic!("persisted private replay")
    };
    assert_eq!(exact, &expected);
    let continuation = read_stored_session_continuation(&reader, child).unwrap();
    let target = continuation.target();
    assert_eq!(
        target.source(),
        Some(BackendResumeSource::InitialFork(JournalSequence::new(2)))
    );
    assert_eq!(
        target.source_initial_fork_sequence(),
        Some(JournalSequence::new(2))
    );
    assert_eq!(target.source_anchor_sequence(), None);
    assert_eq!(target.source_checkpoint_sequence(), None);
    assert_eq!(target.epoch(), 1);
    assert_eq!(target.context_epoch(), Some(1));
    assert_eq!(target.model_replay().items(), expected.items());
    assert_eq!(target.model_replay().contract(), Some(expected.contract()));
    assert_eq!(target.context_policy(), recovered.context_policy());
    assert!(!directory.0.join(format!("{parent}.jsonl")).exists());
}

// seed 하나의 두 group 중 하나만 요약하고 남은 원본 private epoch를 두 checkpoint와 재시작에 걸쳐
// 보존합니다.
#[test]
fn imported_private_groups_survive_two_context_checkpoints_and_restart() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, first_suffix) = fork_private_turn(5, 1, 1);
    let checkpoint = fork_checkpoint(
        1,
        11,
        vec![
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(8),
                JournalSequence::new(10),
                first_suffix.clone(),
            )
            .unwrap(),
        ],
        fork_first_losses(&exact),
        exact.contract(),
    );
    let mut commits = vec![
        initial,
        request,
        complete,
        fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
    ];
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(2));
    let snapshot = decode(&encode(&recovered.complete_snapshot()).unwrap()).unwrap();
    let restarted = recover(slice::from_ref(&snapshot)).unwrap();
    assert_eq!(restarted.model_replay(), recovered.model_replay());
    let (request, complete, second_suffix) = fork_private_turn(13, 2, 2);
    let checkpoint = fork_checkpoint(
        2,
        19,
        vec![
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(12),
                JournalSequence::new(12),
                first_suffix.clone(),
            )
            .unwrap(),
            ContextRetainedGroup::try_new(
                JournalSequence::new(16),
                JournalSequence::new(18),
                second_suffix.clone(),
            )
            .unwrap(),
        ],
        vec![
            ContextLoss::visible_prefix_summarized(
                JournalSequence::new(12),
                JournalSequence::new(12),
            )
            .unwrap(),
        ],
        exact.contract(),
    );
    commits = vec![
        snapshot,
        request,
        complete,
        fork_child_commit(20, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
    ];
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(3));
    let final_snapshot = recovered.complete_snapshot();
    let final_recovery = recover(&[decode(&encode(&final_snapshot).unwrap()).unwrap()]).unwrap();
    let expected = iter::once(ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: fork_portable_summary().into(),
        refusal: None,
    })
    .chain(exact.items()[2..4].iter().cloned())
    .chain(first_suffix)
    .chain(second_suffix)
    .collect::<Vec<_>>();
    assert_eq!(final_recovery.model_replay().items(), expected);
    for record in final_recovery.records() {
        if let JournalRecord::ContextCheckpoint(checkpoint) = record.record() {
            let imported = checkpoint
                .retained_groups()
                .iter()
                .find(|group| group.fork_import().is_some())
                .unwrap();
            assert_eq!(imported.fork_import(), Some((JournalSequence::new(2), 1)));
            assert_eq!(imported.private_epochs(), &[3]);
            assert_eq!(imported.items(), &exact.items()[2..4]);
        }
    }
}

// 같은 seed 참조라도 index·private epoch·payload·group 경계가 달라지면 현재 source와 대조해
// 거부합니다.
#[test]
fn imported_checkpoint_rejects_changed_seed_group_epoch_payload_and_split() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, suffix) = fork_private_turn(5, 1, 1);
    let retained = exact.items()[2..4].to_vec();
    let mut changed = retained.clone();
    let ModelReplayItem::Message { content, .. } = &mut changed[0] else {
        panic!("message")
    };
    *content = "changed".into();
    let invalid = vec![
        ContextRetainedGroup::try_imported(JournalSequence::new(3), 1, retained.clone(), vec![3])
            .unwrap(),
        ContextRetainedGroup::try_imported(JournalSequence::new(2), 0, retained.clone(), vec![3])
            .unwrap(),
        ContextRetainedGroup::try_imported(JournalSequence::new(2), 1, retained.clone(), vec![1])
            .unwrap(),
        ContextRetainedGroup::try_imported(JournalSequence::new(2), 1, changed, vec![3]).unwrap(),
        ContextRetainedGroup::try_imported(
            JournalSequence::new(2),
            1,
            retained[..1].to_vec(),
            vec![],
        )
        .unwrap(),
    ];
    for retained in invalid {
        let checkpoint = fork_checkpoint(
            1,
            11,
            vec![
                retained,
                ContextRetainedGroup::try_new(
                    JournalSequence::new(8),
                    JournalSequence::new(10),
                    suffix.clone(),
                )
                .unwrap(),
            ],
            fork_first_losses(&exact),
            exact.contract(),
        );
        let commits = vec![
            initial.clone(),
            request.clone(),
            complete.clone(),
            fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
        ];
        assert!(recover(&commits).is_err());
    }
}

// 한번 요약으로 제거된 import는 immutable seed에 남아 있어도 후속 checkpoint에서 되살릴 수
// 없습니다.
#[test]
fn summarized_import_cannot_be_resurrected_from_archival_seed() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, suffix) = fork_private_turn(5, 1, 1);
    let first = fork_checkpoint(
        1,
        11,
        vec![
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(8),
                JournalSequence::new(10),
                suffix.clone(),
            )
            .unwrap(),
        ],
        fork_first_losses(&exact),
        exact.contract(),
    );
    let (next_request, next_complete, next_suffix) = fork_private_turn(13, 2, 2);
    let second = fork_checkpoint(
        2,
        19,
        vec![
            fork_import_retained(&exact, 0),
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(12),
                JournalSequence::new(12),
                suffix,
            )
            .unwrap(),
            ContextRetainedGroup::try_new(
                JournalSequence::new(16),
                JournalSequence::new(18),
                next_suffix,
            )
            .unwrap(),
        ],
        vec![
            ContextLoss::visible_prefix_summarized(
                JournalSequence::new(12),
                JournalSequence::new(12),
            )
            .unwrap(),
        ],
        exact.contract(),
    );
    let commits = vec![
        initial,
        request,
        complete,
        fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(first)]),
        next_request,
        next_complete,
        fork_child_commit(20, vec![JournalRecord::ContextCheckpoint(second)]),
    ];
    let error = recover(&commits).unwrap_err();
    assert_eq!(error.commit_index(), Some(6));
}

// seed-only child의 연속 두 번 exact 교체는 현재 owner chain을 따라 같은 baseline·origins·hint를
// 복구합니다.
#[test]
fn two_seed_only_exact_replacements_preserve_imports_after_restart() {
    let (initial, exact) = fork_two_group_bootstrap();
    let commits = vec![
        initial,
        fork_replacement(
            5,
            1,
            fork_exact_transition(),
            "model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
        fork_replacement(
            7,
            2,
            fork_exact_transition(),
            "model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
    ];
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(3));
    assert_eq!(recovered.context_epoch(), Some(1));
    assert_eq!(recovered.initial_fork_seed(), Some(JournalSequence::new(2)));
    assert_eq!(recovered.model_replay().items(), exact.items());
    let restarted =
        recover(&[decode(&encode(&recovered.complete_snapshot()).unwrap()).unwrap()]).unwrap();
    assert_eq!(restarted.binding_epoch(), Some(3));
    assert_eq!(restarted.initial_fork_seed(), Some(JournalSequence::new(2)));
    assert_eq!(restarted.model_replay(), recovered.model_replay());
    let seed = restarted
        .records()
        .iter()
        .find_map(|r| match r.record() {
            JournalRecord::InitialForkSeed(seed) => Some(seed),
            _ => None,
        })
        .unwrap();
    assert_eq!(seed.seed(), &ForkSeed::ExactReplay(exact.clone()));
    assert!(
        exact
            .item_origins()
            .iter()
            .all(|origin| origin.original().binding_epoch() == 3)
    );
    let continuation =
        session_repository::build_continuation(restarted, fixture_session(82)).unwrap();
    let target = continuation.target();
    assert_eq!(
        target.source(),
        Some(BackendResumeSource::InitialFork(JournalSequence::new(2)))
    );
    assert_eq!(
        target.source_initial_fork_sequence(),
        Some(JournalSequence::new(2))
    );
    assert_eq!(target.source_anchor_sequence(), None);
    assert_eq!(target.source_checkpoint_sequence(), None);
    assert_eq!(target.epoch(), 3);
    assert_eq!(target.context_epoch(), Some(1));
    assert!(target.replay_contract_rebind_required());
    assert_eq!(target.model_replay().items(), exact.items());
    assert_eq!(
        target.model_replay_groups(),
        recovered.model_replay_groups()
    );
    assert_eq!(target.context_policy(), recovered.context_policy());
}

// close 없는 owner 변경, 다른 seed/model/private profile은 seed-only exact 교체의 증거가 될 수
// 없습니다.
#[test]
fn seed_only_replacement_rejects_missing_close_source_model_and_profile() {
    let (initial, _) = fork_two_group_bootstrap();
    let replacement = fork_replacement(
        5,
        1,
        fork_exact_transition(),
        "model",
        ReplayProfile::ProviderPrivateLocalPlaintext,
    );
    let no_close = fork_child_commit(5, vec![replacement.records()[1].record().clone()]);
    assert!(recover(&[initial.clone(), no_close]).is_err());
    let wrong_source = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_initial_fork_sequence(JournalSequence::new(3));
    for candidate in [
        fork_replacement(
            5,
            1,
            wrong_source,
            "model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
        fork_replacement(
            5,
            1,
            fork_exact_transition(),
            "changed-model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
        fork_replacement(
            5,
            1,
            fork_exact_transition(),
            "model",
            ReplayProfile::SemanticOnly,
        ),
    ] {
        assert!(recover(&[initial.clone(), candidate]).is_err());
    }
}

// accepted suffix나 checkpoint가 생기면 오래된 initial seed를 새 교체의 reconstruction root로
// 선택하지 못합니다.
#[test]
fn intervening_request_or_checkpoint_disables_seed_only_replacement_source() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, suffix) = fork_private_turn(5, 1, 1);
    assert!(
        recover(&[
            initial.clone(),
            request.clone(),
            fork_replacement(
                8,
                1,
                fork_exact_transition(),
                "model",
                ReplayProfile::ProviderPrivateLocalPlaintext
            )
        ])
        .is_err()
    );
    let checkpoint = fork_checkpoint(
        1,
        11,
        vec![
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(8),
                JournalSequence::new(10),
                suffix,
            )
            .unwrap(),
        ],
        fork_first_losses(&exact),
        exact.contract(),
    );
    let commits = vec![
        initial,
        request,
        complete,
        fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
        fork_replacement(
            13,
            1,
            fork_exact_transition(),
            "model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
    ];
    let error = recover(&commits).unwrap_err();
    assert_eq!(error.commit_index(), Some(4));
}

// completed child Anchor를 source로 교체해도 inherited baseline과 새 local delta가 순서대로 한
// 번씩만 남습니다.
#[test]
fn completed_child_anchor_exact_replacement_preserves_import_and_local_groups() {
    let (initial, _) = fork_two_group_bootstrap();
    let (request, complete, _) = fork_private_turn(5, 1, 1);
    let mut commits = vec![initial, request, complete];
    let before = recover(&commits).unwrap();
    let transition = BindingTransition::new(
        TransitionMode::ExactReplay,
        CacheState::Lost,
        Some(JournalSequence::new(11)),
    );
    commits.push(fork_replacement(
        12,
        1,
        transition,
        "model",
        ReplayProfile::ProviderPrivateLocalPlaintext,
    ));
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.model_replay(), before.model_replay());
    assert_eq!(
        recovered.model_replay_groups(),
        before.model_replay_groups()
    );
    let restarted =
        recover(&[decode(&encode(&recovered.complete_snapshot()).unwrap()).unwrap()]).unwrap();
    assert_eq!(restarted.model_replay(), before.model_replay());
    assert_eq!(restarted.binding_epoch(), Some(2));
}

// checkpoint-root exact 교체 후 새 owner의 turn/checkpoint까지 실행해 원본 private epoch 3 import를
// 유지합니다.
#[test]
fn imported_private_checkpoint_exact_replacement_advances_owner_without_changing_origins() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, suffix) = fork_private_turn(5, 1, 1);
    let checkpoint = fork_checkpoint(
        1,
        11,
        vec![
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(8),
                JournalSequence::new(10),
                suffix.clone(),
            )
            .unwrap(),
        ],
        fork_first_losses(&exact),
        exact.contract(),
    );
    let mut commits = vec![
        initial,
        request,
        complete,
        fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
    ];
    let before = recover(&commits).unwrap();
    let transition = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_checkpoint_sequence(JournalSequence::new(12));
    commits.push(fork_replacement(
        13,
        1,
        transition,
        "model",
        ReplayProfile::ProviderPrivateLocalPlaintext,
    ));
    let replaced = recover(&commits).unwrap();
    assert_eq!(replaced.binding_epoch(), Some(2));
    assert_eq!(replaced.context_epoch(), Some(2));
    assert_eq!(replaced.model_replay(), before.model_replay());
    assert_eq!(replaced.model_replay_groups(), before.model_replay_groups());
    let captured = replaced
        .capture_fork(crate::SessionId::new().unwrap())
        .unwrap();
    assert!(matches!(captured.source(), ForkSource::Checkpoint(_)));
    let ForkSeed::ExactReplay(captured_exact) = captured.seed() else {
        panic!("replacement capture")
    };
    assert_eq!(captured_exact.items(), replaced.model_replay().items());
    for (origin, original) in captured_exact.item_origins()[1..3]
        .iter()
        .zip(&exact.item_origins()[2..4])
    {
        assert_eq!(origin.original(), original.original());
        assert_eq!(origin.source_binding(), original.source_binding());
        assert_eq!(origin.imported_from().session_id(), fixture_session(82));
        assert_eq!(origin.imported_from().binding_epoch(), 1);
        assert_eq!(origin.imported_from().context_epoch(), 2);
        assert_eq!(
            origin.imported_from().record_sequence(),
            JournalSequence::new(12)
        );
    }
    assert_eq!(
        captured_exact.item_origins()[1]
            .imported_from()
            .item_index(),
        1
    );
    assert_eq!(
        captured_exact.item_origins()[2]
            .imported_from()
            .item_index(),
        2
    );
    let snapshot = decode(&encode(&replaced.complete_snapshot()).unwrap()).unwrap();
    let (request, complete, new_suffix) = fork_private_turn_at_epoch(2, 15, 2, 2);
    let checkpoint = fork_checkpoint_at_epoch(
        2,
        2,
        21,
        vec![
            fork_import_retained(&exact, 1),
            ContextRetainedGroup::try_new(
                JournalSequence::new(14),
                JournalSequence::new(14),
                suffix.clone(),
            )
            .unwrap(),
            ContextRetainedGroup::try_new(
                JournalSequence::new(18),
                JournalSequence::new(20),
                new_suffix.clone(),
            )
            .unwrap(),
        ],
        vec![
            ContextLoss::visible_prefix_summarized(
                JournalSequence::new(14),
                JournalSequence::new(14),
            )
            .unwrap(),
        ],
        exact.contract(),
    );
    let final_commits = vec![
        snapshot,
        request,
        complete,
        fork_child_commit(22, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
    ];
    let recovered = recover(&final_commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.context_epoch(), Some(3));
    let expected = iter::once(ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: fork_portable_summary().into(),
        refusal: None,
    })
    .chain(exact.items()[2..4].iter().cloned())
    .chain(suffix)
    .chain(new_suffix)
    .collect::<Vec<_>>();
    let restarted =
        recover(&[decode(&encode(&recovered.complete_snapshot()).unwrap()).unwrap()]).unwrap();
    assert_eq!(restarted.model_replay().items(), expected);
    let checkpoint = restarted
        .records()
        .iter()
        .rev()
        .find_map(|r| match r.record() {
            JournalRecord::ContextCheckpoint(c) => Some(c),
            _ => None,
        })
        .unwrap();
    let imported = &checkpoint.retained_groups()[0];
    assert_eq!(imported.fork_import(), Some((JournalSequence::new(2), 1)));
    assert_eq!(imported.private_epochs(), &[3]);
    let captured = restarted
        .capture_fork(crate::SessionId::new().unwrap())
        .unwrap();
    assert!(matches!(captured.source(), ForkSource::Checkpoint(_)));
    let ForkSeed::ExactReplay(captured_exact) = captured.seed() else {
        panic!("checkpoint capture")
    };
    assert_eq!(captured_exact.items(), restarted.model_replay().items());
    assert_eq!(
        captured_exact.item_origins()[0].original().session_id(),
        fixture_session(82)
    );
    assert_eq!(
        captured_exact.item_origins()[0].original().binding_epoch(),
        2
    );
    assert_eq!(
        captured_exact.item_origins()[0]
            .original()
            .record_sequence(),
        JournalSequence::new(22)
    );
    for (captured_origin, source_origin) in captured_exact.item_origins()[1..3]
        .iter()
        .zip(&exact.item_origins()[2..4])
    {
        assert_eq!(captured_origin.original(), source_origin.original());
        assert_eq!(
            captured_origin.source_binding(),
            source_origin.source_binding()
        );
        assert_eq!(
            captured_origin.imported_from().session_id(),
            fixture_session(82)
        );
        assert_eq!(captured_origin.imported_from().binding_epoch(), 2);
        assert_eq!(captured_origin.imported_from().context_epoch(), 3);
        assert_eq!(
            captured_origin.imported_from().record_sequence(),
            JournalSequence::new(22)
        );
    }
    assert_eq!(
        captured_exact.item_origins()[1]
            .imported_from()
            .item_index(),
        1
    );
    assert_eq!(
        captured_exact.item_origins()[2]
            .imported_from()
            .item_index(),
        2
    );
}

// 모든 import와 local private 내용을 명시적으로 요약하면 보관된 fork ancestry는 새 model과
// semantic-only profile의 checkpoint 기반 교체를 제한하지 않습니다.
#[test]
fn fully_summarized_fork_allows_different_model_semantic_replacement() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, suffix) = fork_private_turn(5, 1, 1);
    let mut losses = vec![
        ContextLoss::visible_prefix_summarized(JournalSequence::new(2), JournalSequence::new(10))
            .unwrap(),
    ];
    for (items, sequence) in [(exact.items(), 2), (suffix.as_slice(), 9)] {
        for item in items {
            if let ModelReplayItem::ProviderPrivateAssistant { envelope } = item {
                losses.push(
                    ContextLoss::provider_private_dropped(
                        envelope.schema(),
                        u64::try_from(envelope.payload().len()).unwrap(),
                        JournalSequence::new(sequence),
                    )
                    .unwrap(),
                );
            }
        }
    }
    let checkpoint = fork_checkpoint(1, 11, vec![], losses, exact.contract());
    let mut commits = vec![
        initial,
        request,
        complete,
        fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
    ];
    let before = recover(&commits).unwrap();
    assert_eq!(before.model_replay().items().len(), 1);
    assert!(
        matches!(&before.model_replay().items()[0], ModelReplayItem::Message { content, .. } if content == fork_portable_summary())
    );
    commits.push(fork_replacement(
        13,
        1,
        BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
            .with_source_checkpoint_sequence(JournalSequence::new(12)),
        "different-model",
        ReplayProfile::SemanticOnly,
    ));
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.model_replay(), before.model_replay());
    let restarted =
        recover(&[decode(&encode(&recovered.complete_snapshot()).unwrap()).unwrap()]).unwrap();
    assert_eq!(restarted.model_replay(), before.model_replay());
    assert_eq!(restarted.binding_epoch(), Some(2));
}

// 활성 import가 남은 새 owner의 첫 delta는 원래 contract를 다시 선언해야 하며 누락하거나
// 다른 system contract로 바꾸면 해당 completion commit을 거부합니다.
#[test]
fn replacement_with_active_import_rejects_missing_or_changed_first_delta_contract() {
    let (initial, _) = fork_two_group_bootstrap();
    let replacement = fork_replacement(
        5,
        1,
        fork_exact_transition(),
        "model",
        ReplayProfile::ProviderPrivateLocalPlaintext,
    );
    let (request, complete, suffix) = fork_private_turn_at_epoch(2, 7, 1, 1);
    assert!(
        recover(&[
            initial.clone(),
            replacement.clone(),
            request.clone(),
            complete.clone()
        ])
        .is_ok()
    );
    for contract in [
        None,
        Some(ModelReplayContract::new("changed system contract", vec![])),
    ] {
        let mut records = complete
            .records()
            .iter()
            .map(|record| record.record().clone())
            .collect::<Vec<_>>();
        records[1] = JournalRecord::ModelReplayDelta(
            ModelReplayDeltaRecord::new(
                2,
                TurnId::new(1.try_into().unwrap()),
                JournalSequence::new(9),
                ModelReplayDelta::new(contract, suffix.clone()),
            )
            .with_context_epoch(1),
        );
        let error = recover(&[
            initial.clone(),
            replacement.clone(),
            request.clone(),
            fork_child_commit(10, records),
        ])
        .unwrap_err();
        assert_eq!(error.commit_index(), Some(3));
    }
}

// seed-only exact child는 가짜 Anchor나 checkpoint 없이 원본 private baseline과 policy로
// 실행 가능한 continuation을 구성합니다.
#[test]
fn seed_only_exact_fork_builds_continuation_without_synthetic_anchor() {
    let (initial, exact) = fork_two_group_bootstrap();
    let recovered = recover(&[initial]).unwrap();
    assert!(recovered.continuation_anchor().is_none());
    assert!(
        !recovered
            .records()
            .iter()
            .any(|record| matches!(record.record(), JournalRecord::ContextCheckpoint(_)))
    );
    let expected_policy = recovered.context_policy().cloned();
    let expected_groups = recovered.model_replay_groups();
    let continuation =
        session_repository::build_continuation(recovered.clone(), fixture_session(82)).unwrap();
    let target = continuation.target();
    assert_eq!(
        target.source(),
        Some(BackendResumeSource::InitialFork(JournalSequence::new(2)))
    );
    assert_eq!(
        target.source_initial_fork_sequence(),
        Some(JournalSequence::new(2))
    );
    assert_eq!(target.source_anchor_sequence(), None);
    assert_eq!(target.source_checkpoint_sequence(), None);
    assert_eq!(target.epoch(), 1);
    assert_eq!(target.context_epoch(), Some(1));
    assert_eq!(target.model_replay().items(), exact.items());
    assert_eq!(target.model_replay().contract(), Some(exact.contract()));
    assert_eq!(target.model_replay_groups(), expected_groups);
    assert_eq!(target.context_policy(), expected_policy.as_ref());
}

// 모든 후속 물리 record를 검증한 뒤 A를 선택하면 B의 입력·private replay·history가 섞이지 않는다.
#[test]
fn historical_fork_selects_exact_older_replay_and_publishes_an_independent_child() {
    use crate::session_repository::{
        SessionForkLimits, StoredSessionForkSourceKind, read_fork_catalog,
    };
    let (initial, inherited) = fork_two_group_bootstrap();
    let (request_a, complete_a, suffix_a) = fork_private_turn(5, 1, 1);
    let (request_b, complete_b, _) = fork_private_turn(12, 2, 1);
    let parent = fixture_session(82);
    let directory = ForkRepositoryDirectory(env::temp_dir().join(format!(
        "yo-historical-fork-{}",
        crate::SessionId::new().unwrap()
    )));
    let mut journal = JournalRepository::new(
        LocalSessionRepository::open(&directory.0, 8 * 1024 * 1024).unwrap(),
    );
    for commit in [&initial, &request_a, &complete_a, &request_b, &complete_b] {
        journal.append(parent, commit).unwrap();
    }
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    let before = reader.read_session(parent).unwrap();
    let catalog = read_fork_catalog(&reader, parent, SessionForkLimits::default()).unwrap();
    assert!(!catalog.truncated());
    assert_eq!(
        catalog
            .boundaries()
            .iter()
            .map(|point| point.journal_cutoff().get())
            .collect::<Vec<_>>(),
        [18, 11, 4]
    );
    let row = &catalog.boundaries()[1];
    assert_eq!(row.source_kind(), StoredSessionForkSourceKind::Anchor);
    assert_eq!(row.logical_cutoff(), JournalSequence::new(10));
    assert_eq!(row.input_excerpt(), Some("child request 1"));
    let selected = catalog.selection(1).unwrap();
    let source = selected
        .prepare_source(parent, catalog.durability())
        .unwrap();
    let expected = inherited
        .items()
        .iter()
        .cloned()
        .chain(suffix_a)
        .collect::<Vec<_>>();
    assert_eq!(source.target().model_replay().items(), expected);
    assert_eq!(
        source.target().source_anchor_sequence(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(source.target().epoch(), 1);
    let child_id = crate::SessionId::new().unwrap();
    let original_binding = source.target().binding();
    let child_binding = BackendBindingEvidence::new(
        original_binding.backend_kind(),
        original_binding.backend_version(),
        original_binding.binding_identity().clone(),
        original_binding.model_identity().clone(),
        BackendIdentity::new("locator/v1", child_id.to_string()),
        original_binding.continuation_strategy(),
    );
    let child = source
        .prepare_exact_fork(fixture_descriptor(child_id), child_binding)
        .unwrap();
    journal.append(child_id, &child.snapshot()).unwrap();
    assert_eq!(reader.read_session(parent).unwrap(), before);
    drop(journal);
    fs::remove_file(directory.0.join(format!("{parent}.jsonl"))).unwrap();
    let child = read_stored_session_continuation(&reader, child_id).unwrap();
    assert_eq!(child.target().model_replay().items(), expected);
    let inherited_history = child.inherited_history().unwrap();
    assert!(inherited_history.sections().iter().all(|section| {
        section
            .records()
            .iter()
            .all(|record| !format!("{record:?}").contains("child request 2"))
    }));
    let stale = crate::JournalDurability::Durable {
        journal_sequence: Some(JournalSequence::new(19)),
        repository_sequence: RepositorySequence::new(6),
    };
    assert!(selected.prepare_source(parent, stale).is_err());
    assert!(
        selected
            .prepare_source(fixture_session(90), catalog.durability())
            .is_err()
    );
}

// 같은 seed의 initial bootstrap과 두 번 교체한 지점은 서로 다른 당시 owner를 복원한다.
#[test]
fn historical_initial_fork_points_keep_the_selected_replacement_owner() {
    let (initial, exact) = fork_two_group_bootstrap();
    let commits = [
        initial,
        fork_replacement(
            5,
            1,
            fork_exact_transition(),
            "model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
        fork_replacement(
            7,
            2,
            fork_exact_transition(),
            "model",
            ReplayProfile::ProviderPrivateLocalPlaintext,
        ),
    ];
    let recovered = recover(&commits).unwrap();
    let (points, truncated) = recovered.historical_fork_boundaries(128).unwrap();
    assert!(!truncated);
    assert_eq!(
        points
            .iter()
            .map(|point| (point.cutoff.get(), point.binding_epoch))
            .collect::<Vec<_>>(),
        [(8, 3), (6, 2), (4, 1)]
    );
    for point in points {
        let prefix = recovered
            .historical_fork_prefix(point.record_count, point.cutoff)
            .unwrap();
        let source = session_repository::build_continuation(prefix, fixture_session(82)).unwrap();
        assert_eq!(source.target().epoch(), point.binding_epoch);
        assert_eq!(source.target().model_replay().items(), exact.items());
        assert_eq!(
            source.target().source_initial_fork_sequence(),
            Some(JournalSequence::new(2))
        );
    }
}

// 과거 checkpoint·Anchor는 최신 model 교체에 종속되지 않고 선택 당시 model/context를 복원한다.
#[test]
fn historical_checkpoint_and_anchor_preserve_their_original_model_and_context() {
    let (initial, exact) = fork_two_group_bootstrap();
    let (request, complete, suffix) = fork_private_turn(5, 1, 1);
    let mut losses = vec![
        ContextLoss::visible_prefix_summarized(JournalSequence::new(2), JournalSequence::new(10))
            .unwrap(),
    ];
    for (items, sequence) in [(exact.items(), 2), (suffix.as_slice(), 9)] {
        for item in items {
            if let ModelReplayItem::ProviderPrivateAssistant { envelope } = item {
                losses.push(
                    ContextLoss::provider_private_dropped(
                        envelope.schema(),
                        envelope.payload().len() as u64,
                        JournalSequence::new(sequence),
                    )
                    .unwrap(),
                );
            }
        }
    }
    let checkpoint = fork_checkpoint(1, 11, vec![], losses, exact.contract());
    let recovered = recover(&[
        initial,
        request,
        complete,
        fork_child_commit(12, vec![JournalRecord::ContextCheckpoint(checkpoint)]),
        fork_replacement(
            13,
            1,
            BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
                .with_source_checkpoint_sequence(JournalSequence::new(12)),
            "different-model",
            ReplayProfile::SemanticOnly,
        ),
    ])
    .unwrap();
    let (points, _) = recovered.historical_fork_boundaries(128).unwrap();
    for (cutoff, model, epoch, context) in [
        (14, "different-model", 2, 2),
        (12, "model", 1, 2),
        (11, "model", 1, 1),
    ] {
        let point = points
            .iter()
            .find(|point| point.cutoff.get() == cutoff)
            .unwrap();
        let prefix = recovered
            .historical_fork_prefix(point.record_count, point.cutoff)
            .unwrap();
        let source = session_repository::build_continuation(prefix, fixture_session(82)).unwrap();
        assert_eq!(source.target().binding().model_identity().value(), model);
        assert_eq!(source.target().epoch(), epoch);
        assert_eq!(source.target().context_epoch(), Some(context));
        if context == 2 {
            assert_eq!(source.target().model_replay().items().len(), 1);
            assert!(
                matches!(&source.target().model_replay().items()[0], ModelReplayItem::Message { content, .. } if content == fork_portable_summary())
            );
        } else {
            assert_eq!(
                source.target().model_replay().items(),
                exact
                    .items()
                    .iter()
                    .cloned()
                    .chain(suffix.clone())
                    .collect::<Vec<_>>()
            );
        }
    }
}

// seed-only child를 연속 분기해도 ultimate private 원본과 history의 source Session을 유지하고
// 부모 파일 없이 각 child bootstrap만으로 복구합니다.
#[test]
fn capture_seed_only_fork_preserves_ultimate_origins_across_generations() {
    let mut records = fork_records(true);
    let JournalRecord::InitialForkSeed(seed) = &records[2] else {
        panic!("seed")
    };
    let parent = fixture_session(81);
    let history = ForkHistoryEntry::new(
        parent,
        ForkHistoryCoordinate::Journal {
            sequence: JournalSequence::new(6),
        },
        SequencedJournalRecord::with_journal_sequence(
            ReplaySequence::new(6),
            JournalSequence::new(6),
            JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                turn: TurnRef::new(parent, TurnId::new(1.try_into().unwrap())),
                outcome: TurnOutcome::Completed,
            }),
        ),
    )
    .unwrap();
    let original_history = history.clone();
    let ForkSeed::ExactReplay(original) = seed.seed() else {
        panic!("exact")
    };
    let original = original.clone();
    records[2] = JournalRecord::InitialForkSeed(Box::new(
        InitialForkSeed::new(
            fixture_session(82),
            parent,
            seed.source().clone(),
            seed.seed().clone(),
            vec![history],
            1024,
        )
        .unwrap(),
    ));
    let recovered = recover(&[fork_commit(records)]).unwrap();
    let grandchild = crate::SessionId::new().unwrap();
    let captured = recovered.capture_fork(grandchild).unwrap();
    assert_eq!(captured.parent_session_id(), fixture_session(82));
    assert!(matches!(captured.source(), ForkSource::InitialFork(_)));
    let ForkSeed::ExactReplay(exact) = captured.seed() else {
        panic!("exact capture")
    };
    assert_eq!(exact.items(), original.items());
    for (origin, original) in exact.item_origins().iter().zip(original.item_origins()) {
        assert_eq!(origin.original(), original.original());
        assert_eq!(origin.source_binding(), original.source_binding());
        assert_eq!(origin.imported_from().session_id(), fixture_session(82));
        assert_eq!(origin.imported_from().binding_epoch(), 1);
        assert_eq!(
            origin.imported_from().record_sequence(),
            JournalSequence::new(2)
        );
    }
    assert!(captured.history().contains(&original_history));
    let commit = fork_captured_bootstrap(grandchild, captured);
    let restarted = recover(&[decode(&encode(&commit).unwrap()).unwrap()]).unwrap();
    let restarted =
        recover(&[decode(&encode(&restarted.complete_snapshot()).unwrap()).unwrap()]).unwrap();
    let next = restarted
        .capture_fork(crate::SessionId::new().unwrap())
        .unwrap();
    assert_eq!(next.parent_session_id(), grandchild);
    let ForkSeed::ExactReplay(exact) = next.seed() else {
        panic!("recaptured exact")
    };
    assert_eq!(exact.items(), original.items());
    for (origin, original) in exact.item_origins().iter().zip(original.item_origins()) {
        assert_eq!(origin.original(), original.original());
        assert_eq!(origin.source_binding(), original.source_binding());
        assert_eq!(origin.imported_from().session_id(), grandchild);
    }
    assert!(next.history().contains(&original_history));
}
