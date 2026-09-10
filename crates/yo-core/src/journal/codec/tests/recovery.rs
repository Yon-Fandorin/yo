use std::{slice, time::Duration};

use super::{
    AgentCommand, AgentEvent, JournalCommit, JournalRecord, MessageOutcome, MessageSegment,
    MessageSegmenter, MessageStream, activity, descriptor_with_path, encode, recover, sequenced,
    submission,
};
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, BackendBindingEvidence,
    BackendIdentity, BackendResumeSource, ContinuationStrategy, JournalSequence,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile, TurnId, TurnOutcome, TurnRef,
    fixture_descriptor, fixture_session,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, BindingCloseReason, BindingTransition,
        CacheState, ContextCheckpoint, ContextLoss, ContextPolicyChanged, ContextRetainedGroup,
        ContextStrategy, ContextSummaryUsage, ContinuationAnchor, DetailAvailability,
        ExchangeDirection, ExchangeKind, ForkExactReplay, ForkGroup, ForkHistoryCoordinate,
        ForkHistoryEntry, ForkItemCoordinate, ForkItemOrigin, ForkSeed, ForkSource,
        ForkSourcePoint, InitialForkSeed, ModelReplayDeltaRecord, OperationId, ReplaySequence,
        SequencedJournalRecord, TransitionMode, VersionedIdentity, decode,
    },
    provider_private_schema,
    session_repository::{
        LocalSessionReader, LocalSessionRepository, StoredSessionReader,
        journal::JournalRepository, read_stored_session, read_stored_session_continuation,
    },
};

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
            crate::journal::CommittedCommand::submission(command.clone(), submission(9)).unwrap(),
        )],
    ));
    let duplicate = JournalCommit::incremental(sequenced(
        3,
        [JournalRecord::CommandCommitted(
            crate::journal::CommittedCommand::submission(command, submission(9)).unwrap(),
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
                crate::journal::CommittedCommand::uncorrelated(AgentCommand::CreateSession {
                    session_id: first,
                })
                .unwrap(),
            ),
            JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: second }),
        ],
    ));

    let error = encode(&commit).expect_err("one commit cannot cross Session ownership");

    assert!(error.to_string().contains("different Sessions"));
}

// backend TextDelta를 일반 AgentEvent로 durable codec에 넣으면 transport chunk가 replay
// authority가 되므로 bounded MessageSegment 경계를 우회하는 text event를 거부해야 한다.
#[test]
fn rejects_raw_text_updates_that_bypass_message_segments() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity: activity(),
            update: ActivityUpdate::TextDelta("raw delta".to_owned()),
        })],
    ));

    let error = encode(&commit).expect_err("raw backend text is not durable semantics");

    assert!(error.to_string().contains("bounded MessageSegments"));
}

// 완결 seal의 segment 수가 durable prefix와 다르면 손상된 message를 completed로
// 재생하지 않고 복구 오류로 거부해야 한다.
#[test]
fn rejects_a_terminal_seal_that_does_not_match_durable_segments() {
    let activity = activity();
    let mut segmenter = MessageSegmenter::new(activity, MessageStream::Agent);
    let segment = segmenter
        .push_text(&"x".repeat(16 * 1024), Duration::ZERO)
        .pop()
        .expect("the size boundary emits a segment");
    let mismatched = super::MessageEnded::new(
        activity,
        MessageStream::Agent,
        MessageOutcome::Completed,
        2,
        16 * 1024,
    );
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::MessageSegment(segment),
            JournalRecord::MessageEnded(super::MessageTerminal::new(None, mismatched)),
        ],
    ));

    let error = recover(&[commit]).expect_err("the inconsistent seal is corruption");

    assert!(error.to_string().contains("does not match"));
}

// crash로 마지막 MessageEnded만 없는 durable message를 복구하면 다음 sequence에
// interrupted seal을 제안해 partial임을 보존하고 completed로 추정하지 않아야 한다.
#[test]
fn recovery_seals_an_unterminated_message_as_interrupted() {
    let activity = activity();
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity,
            MessageStream::Agent,
            1,
            "partial".to_owned(),
        ))],
    ));

    let recovered = recover(&[commit]).expect("the durable prefix recovers");
    let seal = recovered
        .recovery_commit()
        .expect("an unterminated message requires a recovery seal");
    let JournalRecord::MessageEnded(terminal) = seal.records()[0].record() else {
        panic!("recovery emits a terminal seal");
    };
    let ended = terminal.ended();

    assert_eq!(seal.records()[0].sequence().get(), 2);
    assert_eq!(ended.outcome(), &MessageOutcome::Interrupted);
    assert_eq!(ended.segment_count(), 1);
    assert_eq!(ended.utf8_bytes(), 7);
    assert_eq!(recovered.records().len(), 1);
    let snapshot = recovered.complete_snapshot();
    assert_eq!(snapshot.records().len(), 2);
    assert_eq!(snapshot.records()[0].sequence().get(), 1);
    assert_eq!(snapshot.records()[1].sequence().get(), 2);
}

// non-text 경계에서 pending text가 segment로 먼저 강제 저장되면 message가 아직 끝나지
// 않았더라도 다른 Activity 사건을 기록할 수 있어야 한다. 재시작 시에는 마지막 durable
// 위치에서 열린 message만 interrupted로 봉인해 동시 Activity의 원래 순서를 보존한다.
#[test]
fn recovery_preserves_an_event_after_a_forced_message_segment() {
    let activity = activity();
    let message = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity,
            MessageStream::Agent,
            1,
            "partial".to_owned(),
        ))],
    ));
    let later_event = JournalCommit::incremental(sequenced(
        2,
        [JournalRecord::EventCommitted(AgentEvent::TurnFinished {
            turn: activity.turn(),
            outcome: TurnOutcome::Interrupted,
        })],
    ));

    let recovered = recover(&[message, later_event]).expect("the ordering boundary is replayable");

    assert_eq!(recovered.records().len(), 2);
    assert!(recovered.recovery_commit().is_some());
}

// ActivityStarted 뒤 첫 text segment가 저장되기 전에 crash가 나도 durable activity 자체가
// 열린 zero-byte message를 증명한다. 복구는 이를 completed로 꾸미지 않고 interrupted
// MessageEnded(0 segments, 0 bytes)로 봉인해야 한다.
#[test]
fn recovery_seals_a_started_message_before_its_first_text() {
    let activity = activity();
    let started = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        })],
    ));

    let recovered = recover(&[started]).expect("the zero-byte live message is recoverable");
    let seal = recovered
        .recovery_commit()
        .expect("the crash leaves one interrupted seal");
    let JournalRecord::MessageEnded(terminal) = seal.records()[0].record() else {
        panic!("recovery emits a typed message terminal");
    };
    assert_eq!(terminal.ended().segment_count(), 0);
    assert_eq!(terminal.ended().utf8_bytes(), 0);
    assert_eq!(terminal.ended().outcome(), &MessageOutcome::Interrupted);
}

// 같은 message에 두 번째 MessageEnded가 나타나면 각 seal이 0-byte 완료처럼 보이더라도
// terminal identity가 모호해지므로 recovery가 중복 종료를 손상으로 거부해야 한다.
#[test]
fn recovery_rejects_a_duplicate_terminal_seal() {
    let ended = super::MessageEnded::new(
        activity(),
        MessageStream::Agent,
        MessageOutcome::Completed,
        0,
        0,
    );
    let commit = JournalCommit::incremental(sequenced(
        1,
        [
            JournalRecord::MessageEnded(super::MessageTerminal::new(None, ended.clone())),
            JournalRecord::MessageEnded(super::MessageTerminal::new(None, ended)),
        ],
    ));

    let error = recover(&[commit]).expect_err("a duplicate terminal seal is corruption");

    assert!(error.to_string().contains("more than one"));
}

fn fork_binding(profile: ReplayProfile) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "managed",
        "1.0.0",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "source"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: profile,
        },
    )
}

fn fork_records(private: bool) -> Vec<JournalRecord> {
    let parent = fixture_session(81);
    let child = fixture_session(82);
    let profile = if private {
        ReplayProfile::ProviderPrivateLocalPlaintext
    } else {
        ReplayProfile::SemanticOnly
    };
    let source = fork_binding(profile);
    let mut items = vec![ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: "exact\r\nvisible".into(),
        refusal: None,
    }];
    if private {
        items.push(ModelReplayItem::ProviderPrivateAssistant {
            envelope: ProviderPrivateReplayEnvelope::new(
                provider_private_schema(profile).unwrap(),
                br#"{"reasoning_content":"private\r\nbytes","content":"exact\r\nvisible"}"#
                    .to_vec(),
            )
            .unwrap(),
        });
    }
    let origins = (0..items.len())
        .map(|index| {
            let coordinate =
                ForkItemCoordinate::new(parent, 3, 2, JournalSequence::new(7), index).unwrap();
            ForkItemOrigin::new(coordinate, coordinate, source.clone()).unwrap()
        })
        .collect();
    let groups = vec![ForkGroup::new(0, items.len()).unwrap()];
    let exact = ForkExactReplay::new(
        ModelReplayContract::new("exact system contract", vec![]),
        items,
        origins,
        groups,
    )
    .unwrap();
    let seed = InitialForkSeed::new(
        child,
        parent,
        ForkSource::Anchor(
            ForkSourcePoint::new(
                3,
                2,
                JournalSequence::new(9),
                JournalSequence::new(8),
                source.clone(),
            )
            .unwrap(),
        ),
        ForkSeed::ExactReplay(exact),
        vec![],
        2,
    )
    .unwrap();
    vec![
        JournalRecord::SessionDescriptor(fixture_descriptor(child)),
        JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child }),
        JournalRecord::InitialForkSeed(Box::new(seed)),
        JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
            1,
            source.backend_kind(),
            source.backend_version(),
            VersionedIdentity::new(
                source.binding_identity().schema(),
                source.binding_identity().value(),
            ),
            VersionedIdentity::new(
                source.model_identity().schema(),
                source.model_identity().value(),
            ),
            VersionedIdentity::new("locator/v1", "child"),
            BindingTransition::initial_fork(JournalSequence::new(2)),
            source.continuation_strategy(),
        )),
        JournalRecord::ContextPolicyChanged(
            ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65_536),
            )
            .unwrap(),
        ),
    ]
}

fn fork_commit(records: Vec<JournalRecord>) -> JournalCommit {
    let cutoff = JournalSequence::new((records.len() - 1) as u64);
    let entries = records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            let sequence = ReplaySequence::new(index as u64 + 1);
            if index == 0 {
                SequencedJournalRecord::storage(sequence, record)
            } else {
                SequencedJournalRecord::with_journal_sequence(
                    sequence,
                    JournalSequence::new(index as u64),
                    record,
                )
            }
        })
        .collect();
    JournalCommit::incremental_through(cutoff, entries)
}

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

fn fork_child_commit(first: u64, records: Vec<JournalRecord>) -> JournalCommit {
    let cutoff = first + records.len() as u64 - 1;
    JournalCommit::incremental_through(
        JournalSequence::new(cutoff),
        records
            .into_iter()
            .enumerate()
            .map(|(index, record)| {
                let sequence = first + index as u64;
                SequencedJournalRecord::with_journal_sequence(
                    ReplaySequence::new(sequence + 1),
                    JournalSequence::new(sequence),
                    record,
                )
            })
            .collect(),
    )
}

fn fork_child_request() -> (TurnRef, JournalCommit) {
    let turn = TurnRef::new(fixture_session(82), TurnId::new(1.try_into().unwrap()));
    let submission_id = submission(83);
    let operation = OperationId::from(submission_id);
    let request = fork_child_commit(
        5,
        vec![
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("child followup"),
                    },
                    submission_id,
                )
                .unwrap(),
            ),
            JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                1,
                operation,
                ExchangeKind::Request,
                ExchangeDirection::YoToBackend,
                "managed.request/v1",
                None,
                None,
                DetailAvailability::Unpersisted,
            )),
            JournalRecord::BackendRequestAccepted(
                BackendRequestAccepted::new(
                    1,
                    turn.turn_id(),
                    operation,
                    JournalSequence::new(6),
                    VersionedIdentity::new("request/v1", "child-request"),
                )
                .with_context_epoch(1),
            ),
        ],
    );
    (turn, request)
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
    assert!(crate::session_repository::build_continuation(recovered, fixture_session(82)).is_err());
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
        assert!(
            crate::session_repository::build_continuation(reloaded, fixture_session(82)).is_ok()
        );
    }
}

struct ForkRepositoryDirectory(std::path::PathBuf);

impl Drop for ForkRepositoryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// 부모 파일 없이 child만 디스크에 저장·재개하고 inherited usage를 child 사용량으로 합산하지
// 않습니다.
#[test]
fn fork_repository_reopens_private_seed_without_ancestor_files_or_inherited_usage() {
    let directory = ForkRepositoryDirectory(std::env::temp_dir().join(format!(
        "yo-fork-independent-{}",
        crate::SessionId::new().unwrap()
    )));
    std::fs::create_dir(&directory.0).unwrap();
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
        crate::session_repository::SessionUsageProjection::from_records(&source_projection)
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

fn fork_private_answer(text: &str) -> Vec<ModelReplayItem> {
    vec![ModelReplayItem::Message { role: ModelReplayRole::Assistant, content: text.into(), refusal: None },
        ModelReplayItem::ProviderPrivateAssistant { envelope: ProviderPrivateReplayEnvelope::new(
            provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
            serde_json::to_vec(&serde_json::json!({"reasoning_content":format!("private {text}"),"content":text})).unwrap()).unwrap() }]
}

fn fork_two_group_bootstrap() -> (JournalCommit, ForkExactReplay) {
    let mut records = fork_records(true);
    let JournalRecord::InitialForkSeed(seed) = &records[2] else {
        panic!("seed")
    };
    let ForkSeed::ExactReplay(first) = seed.seed() else {
        panic!("exact")
    };
    let mut items = first.items().to_vec();
    items.extend(fork_private_answer("keep imported group one"));
    let origins = (0..items.len())
        .map(|index| {
            let coordinate =
                ForkItemCoordinate::new(fixture_session(81), 3, 2, JournalSequence::new(7), index)
                    .unwrap();
            ForkItemOrigin::new(
                coordinate,
                coordinate,
                fork_binding(ReplayProfile::ProviderPrivateLocalPlaintext),
            )
            .unwrap()
        })
        .collect();
    let exact = ForkExactReplay::new(
        first.contract().clone(),
        items,
        origins,
        vec![ForkGroup::new(0, 2).unwrap(), ForkGroup::new(2, 4).unwrap()],
    )
    .unwrap();
    records[2] = JournalRecord::InitialForkSeed(Box::new(
        InitialForkSeed::new(
            fixture_session(82),
            fixture_session(81),
            seed.source().clone(),
            ForkSeed::ExactReplay(exact.clone()),
            vec![],
            2,
        )
        .unwrap(),
    ));
    (fork_commit(records), exact)
}

fn fork_private_turn(
    first: u64,
    turn_id: u64,
    context_epoch: u64,
) -> (JournalCommit, JournalCommit, Vec<ModelReplayItem>) {
    fork_private_turn_at_epoch(1, first, turn_id, context_epoch)
}

fn fork_private_turn_at_epoch(
    epoch: u64,
    first: u64,
    turn_id: u64,
    context_epoch: u64,
) -> (JournalCommit, JournalCommit, Vec<ModelReplayItem>) {
    let turn = TurnRef::new(
        fixture_session(82),
        TurnId::new(turn_id.try_into().unwrap()),
    );
    let submission_id = submission(u8::try_from(90 + turn_id).unwrap());
    let operation = OperationId::from(submission_id);
    let input = format!("child request {turn_id}");
    let request = fork_child_commit(
        first,
        vec![
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new(input.clone()),
                    },
                    submission_id,
                )
                .unwrap(),
            ),
            JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                epoch,
                operation,
                ExchangeKind::Request,
                ExchangeDirection::YoToBackend,
                "managed.request/v1",
                None,
                None,
                DetailAvailability::Unpersisted,
            )),
            JournalRecord::BackendRequestAccepted(
                BackendRequestAccepted::new(
                    epoch,
                    turn.turn_id(),
                    operation,
                    JournalSequence::new(first + 1),
                    VersionedIdentity::new("request/v1", format!("request-{turn_id}")),
                )
                .with_context_epoch(context_epoch),
            ),
        ],
    );
    let mut suffix = vec![ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: input,
        refusal: None,
    }];
    suffix.extend(fork_private_answer(&format!("child answer {turn_id}")));
    let complete = fork_child_commit(
        first + 3,
        vec![
            JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                turn,
                outcome: TurnOutcome::Completed,
            }),
            JournalRecord::ModelReplayDelta(
                ModelReplayDeltaRecord::new(
                    epoch,
                    turn.turn_id(),
                    JournalSequence::new(first + 2),
                    ModelReplayDelta::new(
                        (epoch > 1)
                            .then(|| ModelReplayContract::new("exact system contract", vec![])),
                        suffix.clone(),
                    ),
                )
                .with_context_epoch(context_epoch),
            ),
            JournalRecord::BackendResumableOutcome(
                BackendResumableOutcome::new(
                    epoch,
                    turn.turn_id(),
                    JournalSequence::new(first + 2),
                    Some(VersionedIdentity::new(
                        "outcome/v1",
                        format!("outcome-{turn_id}"),
                    )),
                    Some(JournalSequence::new(first + 4)),
                )
                .with_context_epoch(context_epoch),
            ),
            JournalRecord::ContinuationAnchor(
                ContinuationAnchor::new(
                    epoch,
                    JournalSequence::new(first + 2),
                    JournalSequence::new(first + 5),
                    JournalSequence::new(first + 5),
                )
                .with_context_epoch(context_epoch),
            ),
        ],
    );
    (request, complete, suffix)
}

fn fork_portable_summary() -> &'static str {
    "# Context Checkpoint\n## Current Objective\nContinue.\n## Active Constraints\nNone.\n## Decisions\nNone.\n## Verified Progress\nEarlier group summarized.\n## Current State\nIdle.\n## Unknown or Unverified\nNone.\n## Next Actions\nContinue.\n## Critical References\nNone."
}

fn fork_checkpoint(
    previous: u64,
    anchor: u64,
    groups: Vec<ContextRetainedGroup>,
    losses: Vec<ContextLoss>,
    contract: &ModelReplayContract,
) -> ContextCheckpoint {
    fork_checkpoint_at_epoch(1, previous, anchor, groups, losses, contract)
}

fn fork_checkpoint_at_epoch(
    epoch: u64,
    previous: u64,
    anchor: u64,
    groups: Vec<ContextRetainedGroup>,
    losses: Vec<ContextLoss>,
    contract: &ModelReplayContract,
) -> ContextCheckpoint {
    let usage = ContextSummaryUsage::try_new(serde_json::json!({"schema":"yo.model-usage-receipt/v1","response_id":"summary","round":1,
        "provider":"test","account":"default","model":"test","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://example.invalid/",
        "usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"reasoning_tokens":0},"cache_read_input_tokens":{"availability":"unsupported"}})).unwrap();
    let first = groups.first().map(ContextRetainedGroup::first_sequence);
    ContextCheckpoint::try_new(
        epoch,
        previous,
        previous + 1,
        JournalSequence::new(anchor),
        JournalSequence::new(anchor),
        1,
        ContextStrategy::PortableSummaryV1Alpha1,
        100_000,
        90_000,
        20_000,
        contract.clone(),
        fork_portable_summary(),
        groups,
        first,
        vec![],
        losses,
        usage,
    )
    .unwrap()
}

fn fork_import_retained(exact: &ForkExactReplay, index: usize) -> ContextRetainedGroup {
    let group = exact.groups()[index];
    ContextRetainedGroup::try_imported(
        JournalSequence::new(2),
        index,
        exact.items()[group.first_item()..group.end_item()].to_vec(),
        vec![3],
    )
    .unwrap()
}

fn fork_first_losses(exact: &ForkExactReplay) -> Vec<ContextLoss> {
    let ModelReplayItem::ProviderPrivateAssistant { envelope } = &exact.items()[1] else {
        panic!("private")
    };
    vec![
        ContextLoss::visible_prefix_summarized(JournalSequence::new(2), JournalSequence::new(2))
            .unwrap(),
        ContextLoss::provider_private_dropped(
            envelope.schema(),
            envelope.payload().len() as u64,
            JournalSequence::new(2),
        )
        .unwrap(),
    ]
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
    let expected = std::iter::once(ModelReplayItem::Message {
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

fn fork_exact_transition() -> BindingTransition {
    BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_initial_fork_sequence(JournalSequence::new(2))
}

fn fork_replacement(
    first: u64,
    previous: u64,
    transition: BindingTransition,
    model: &str,
    profile: ReplayProfile,
) -> JournalCommit {
    let strategy = fork_binding(profile).continuation_strategy();
    fork_child_commit(
        first,
        vec![
            JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                previous,
                BindingCloseReason::Replaced,
            )),
            JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                previous + 1,
                "managed",
                "1.0.0",
                VersionedIdentity::new("binding/v1", "account"),
                VersionedIdentity::new("model/v1", model),
                VersionedIdentity::new("locator/v1", format!("child-epoch-{}", previous + 1)),
                transition,
                strategy,
            )),
        ],
    )
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
        crate::session_repository::build_continuation(restarted, fixture_session(82)).unwrap();
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
    let expected = std::iter::once(ModelReplayItem::Message {
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
        crate::session_repository::build_continuation(recovered.clone(), fixture_session(82))
            .unwrap();
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

fn fork_captured_bootstrap(child: crate::SessionId, seed: InitialForkSeed) -> JournalCommit {
    let mut records = fork_records(true);
    records[0] = JournalRecord::SessionDescriptor(fixture_descriptor(child));
    records[1] = JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child });
    records[2] = JournalRecord::InitialForkSeed(Box::new(seed));
    fork_commit(records)
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
    let directory = ForkRepositoryDirectory(std::env::temp_dir().join(format!(
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
    std::fs::remove_file(directory.0.join(format!("{parent}.jsonl"))).unwrap();
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
        repository_sequence: crate::session_repository::RepositorySequence::new(6),
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
        let source =
            crate::session_repository::build_continuation(prefix, fixture_session(82)).unwrap();
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
        let source =
            crate::session_repository::build_continuation(prefix, fixture_session(82)).unwrap();
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

// 표시 cap과 물리 검증 cap을 분리해 첫 초과·후반 discovery 손상을 prefix 성공으로 숨기지 않는다.
#[test]
fn historical_catalog_enforces_full_physical_bounds_and_late_discovery_validation() {
    use crate::session_repository::{
        DurableRecord, RecordDiscovery, SessionForkLimits, SessionRepository, read_fork_catalog,
    };
    let parent = fixture_session(82);
    let directory = ForkRepositoryDirectory(std::env::temp_dir().join(format!(
        "yo-historical-bounds-{}",
        crate::SessionId::new().unwrap()
    )));
    let (initial, _) = fork_two_group_bootstrap();
    let (request, complete, _) = fork_private_turn(5, 1, 1);
    let mut journal = JournalRepository::new(
        LocalSessionRepository::open(&directory.0, 8 * 1024 * 1024).unwrap(),
    );
    for commit in [&initial, &request, &complete] {
        journal.append(parent, commit).unwrap();
    }
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    let bytes = std::fs::metadata(directory.0.join(format!("{parent}.jsonl")))
        .unwrap()
        .len();
    let exact_limits = SessionForkLimits::try_new(bytes, 3, 1).unwrap();
    let catalog = read_fork_catalog(&reader, parent, exact_limits).unwrap();
    assert!(catalog.truncated());
    assert_eq!(catalog.boundaries().len(), 1);
    assert_eq!(
        catalog.boundaries()[0].journal_cutoff(),
        JournalSequence::new(11)
    );
    assert!(
        catalog
            .selection(0)
            .unwrap()
            .prepare_source(parent, catalog.durability())
            .is_ok()
    );
    for limits in [
        SessionForkLimits::try_new(bytes - 1, 3, 1).unwrap(),
        SessionForkLimits::try_new(bytes, 2, 1).unwrap(),
    ] {
        assert!(read_fork_catalog(&reader, parent, limits).is_err());
    }
    let recovered = recover(&[initial, request, complete]).unwrap();
    let mut repository = journal.into_inner();
    repository
        .append(
            parent,
            DurableRecord::snapshot(encode(&recovered.complete_snapshot()).unwrap())
                .with_journal_cutoff(recovered.journal_cutoff())
                .with_discovery(
                    RecordDiscovery::new(fixture_descriptor(parent)).with_binding_epoch(99),
                ),
        )
        .unwrap();
    assert!(
        read_fork_catalog(&reader, parent, SessionForkLimits::default())
            .unwrap_err()
            .to_string()
            .contains("discovery")
    );
}

// 최신 parent가 accepted suffix를 남기면 정상인 과거 initial seed도 선택 가능한 목록이 되지 않는다.
#[test]
fn historical_catalog_rejects_a_latest_uncertain_parent() {
    use crate::session_repository::{SessionForkLimits, read_fork_catalog};
    let parent = fixture_session(82);
    let directory = ForkRepositoryDirectory(std::env::temp_dir().join(format!(
        "yo-historical-uncertain-{}",
        crate::SessionId::new().unwrap()
    )));
    let (initial, _) = fork_two_group_bootstrap();
    let (request, _, _) = fork_private_turn(5, 1, 1);
    let mut journal = JournalRepository::new(
        LocalSessionRepository::open(&directory.0, 8 * 1024 * 1024).unwrap(),
    );
    journal.append(parent, &initial).unwrap();
    journal.append(parent, &request).unwrap();
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    assert!(read_fork_catalog(&reader, parent, SessionForkLimits::default()).is_err());
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

// command가 진행 중이거나 request가 수락된 뒤 완료 증거가 없는 parent는 오래된 seed를
// 사용해 실행 가능한 child를 캡처하지 못합니다.
#[test]
fn capture_fork_rejects_active_and_uncertain_suffix() {
    let (initial, _) = fork_two_group_bootstrap();
    let (_, request) = fork_child_request();
    let active = fork_child_commit(5, vec![request.records()[0].record().clone()]);
    for suffix in [active, request] {
        let recovered = recover(&[initial.clone(), suffix]).unwrap();
        assert!(
            recovered
                .capture_fork(crate::SessionId::new().unwrap())
                .is_err()
        );
    }
}

// 상속 history와 현재 parent의 SessionCreated를 합쳐 4096개까지 캡처하고 첫 초과 항목을
// 조용히 자르지 않고 거부합니다.
#[test]
fn capture_fork_history_accepts_limit_and_rejects_first_inherited_excess() {
    for inherited_count in [4095_usize, 4096] {
        let mut records = fork_records(true);
        let JournalRecord::InitialForkSeed(seed) = &records[2] else {
            panic!("seed")
        };
        let parent = fixture_session(81);
        let history = (1..=inherited_count)
            .map(|index| {
                let sequence = JournalSequence::new(u64::try_from(index).unwrap());
                ForkHistoryEntry::new(
                    parent,
                    ForkHistoryCoordinate::Journal { sequence },
                    SequencedJournalRecord::with_journal_sequence(
                        ReplaySequence::new(sequence.get()),
                        sequence,
                        JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                            turn: TurnRef::new(
                                parent,
                                TurnId::new(sequence.get().try_into().unwrap()),
                            ),
                            outcome: TurnOutcome::Completed,
                        }),
                    ),
                )
                .unwrap()
            })
            .collect();
        let source = ForkSource::Anchor(
            ForkSourcePoint::new(
                3,
                2,
                JournalSequence::new(5001),
                JournalSequence::new(5000),
                seed.source().point().unwrap().binding().clone(),
            )
            .unwrap(),
        );
        records[2] = JournalRecord::InitialForkSeed(Box::new(
            InitialForkSeed::new(
                fixture_session(82),
                parent,
                source,
                seed.seed().clone(),
                history,
                1024 * 1024,
            )
            .unwrap(),
        ));
        let initial = fork_commit(records);
        let recovered = recover(&[decode(&encode(&initial).unwrap()).unwrap()]).unwrap();
        let captured = recovered.capture_fork(crate::SessionId::new().unwrap());
        if inherited_count == 4095 {
            let captured = captured.unwrap();
            assert_eq!(captured.history().len(), 4096);
            assert!(
                captured.history()[..inherited_count]
                    .iter()
                    .all(|entry| entry.source_session_id() == parent)
            );
            let appended = captured.history().last().unwrap();
            assert_eq!(appended.source_session_id(), fixture_session(82));
            assert!(
                matches!(appended.record().record(), JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id }) if *session_id == fixture_session(82))
            );
        } else {
            assert!(
                captured
                    .unwrap_err()
                    .to_string()
                    .contains("fork history item limit exceeded")
            );
        }
    }
}
