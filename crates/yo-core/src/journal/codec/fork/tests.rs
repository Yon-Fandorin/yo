use super::{
    super::{
        JournalRecord, MessageReset, MessageStream, ReplaySequence, SequencedJournalRecord,
        VersionedIdentity,
    },
    ForkExactReplay, ForkGroup, ForkHistoryCoordinate, ForkHistoryEntry, ForkItemCoordinate,
    ForkItemOrigin, ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint,
    INITIAL_FORK_SEED_PROFILE, InitialForkSeed,
};
use crate::{
    ActivityId, ActivityRef, AgentEvent, BackendBindingEvidence, BackendIdentity,
    ContinuationStrategy, JournalSequence, ModelReplayContract, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile, SessionId, TurnId, TurnRef,
};

fn session(value: u8) -> SessionId {
    format!("01900000-0000-7000-8000-{value:012}")
        .parse()
        .unwrap()
}

fn binding(profile: ReplayProfile) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "managed",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "parent"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: profile,
        },
    )
}

fn point(record: u64, boundary: u64, binding: BackendBindingEvidence) -> ForkSourcePoint {
    ForkSourcePoint::new(
        3,
        2,
        JournalSequence::new(record),
        JournalSequence::new(boundary),
        binding,
    )
    .unwrap()
}

fn item() -> ModelReplayItem {
    ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "exact\r\nbytes".into(),
        refusal: None,
    }
}

fn origin(parent: SessionId, index: usize, binding: BackendBindingEvidence) -> ForkItemOrigin {
    let coordinate = ForkItemCoordinate::new(parent, 3, 2, JournalSequence::new(7), index).unwrap();
    ForkItemOrigin::new(coordinate, coordinate, binding).unwrap()
}

fn exact(parent: SessionId) -> ForkExactReplay {
    ForkExactReplay::new(
        ModelReplayContract::new("system", vec![]),
        vec![item()],
        vec![origin(parent, 0, binding(ReplayProfile::SemanticOnly))],
        vec![ForkGroup::new(0, 1).unwrap()],
    )
    .unwrap()
}

// root와 child의 identity를 분리하고 empty fork가 inherited context를 숨기지 못하게 합니다.
#[test]
fn empty_requires_distinct_child_and_empty_source_history() {
    assert_eq!(INITIAL_FORK_SEED_PROFILE, "yo.session-fork-seed/v1");
    assert!(
        InitialForkSeed::new(
            session(1),
            session(1),
            ForkSource::Empty,
            ForkSeed::Empty,
            vec![],
            2
        )
        .is_err()
    );
    assert!(
        InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::Empty,
            ForkSeed::Empty,
            vec![],
            2
        )
        .is_ok()
    );
    assert!(
        InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::Empty,
            ForkSeed::ExactReplay(exact(session(1))),
            vec![],
            2
        )
        .is_err()
    );
}

// reset record를 segment 좌표로 위장하거나 종료되지 않은 메시지를 seed로 가져올 수 없습니다.
#[test]
fn message_coordinate_matches_record_and_requires_terminal_seal() {
    let activity = ActivityRef::new(
        TurnRef::new(session(1), TurnId::new(1.try_into().unwrap())),
        ActivityId::new(1.try_into().unwrap()),
    );
    let record = SequencedJournalRecord::storage(
        ReplaySequence::new(1),
        JournalRecord::MessageReset(MessageReset::new(activity, MessageStream::Agent, 1)),
    );
    for part in [ForkMessagePart::Segment, ForkMessagePart::Ended] {
        assert!(
            ForkHistoryEntry::new(
                session(1),
                ForkHistoryCoordinate::Message {
                    activity,
                    part,
                    ordinal: 0
                },
                record.clone()
            )
            .is_err()
        );
    }
    let entry = ForkHistoryEntry::new(
        session(1),
        ForkHistoryCoordinate::Message {
            activity,
            part: ForkMessagePart::Reset,
            ordinal: 0,
        },
        record,
    )
    .unwrap();
    assert!(
        InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::Anchor(point(9, 8, binding(ReplayProfile::SemanticOnly))),
            ForkSeed::ExactReplay(exact(session(1))),
            vec![entry],
            256
        )
        .is_err()
    );
}

// Anchor/outcome, checkpoint, initial seed의 경계를 서로 바꾸어 사용할 수 없습니다.
#[test]
fn source_kinds_require_their_own_boundary_relationship() {
    for source in [
        ForkSource::Anchor(point(8, 8, binding(ReplayProfile::SemanticOnly))),
        ForkSource::Checkpoint(point(8, 7, binding(ReplayProfile::SemanticOnly))),
        ForkSource::InitialFork(point(8, 8, binding(ReplayProfile::SemanticOnly))),
    ] {
        assert!(
            InitialForkSeed::new(
                session(2),
                session(1),
                source,
                ForkSeed::ExactReplay(exact(session(1))),
                vec![],
                2
            )
            .is_err()
        );
    }
    for source in [
        ForkSource::Anchor(point(9, 8, binding(ReplayProfile::SemanticOnly))),
        ForkSource::Checkpoint(point(8, 8, binding(ReplayProfile::SemanticOnly))),
        ForkSource::InitialFork(point(6, 8, binding(ReplayProfile::SemanticOnly))),
    ] {
        let seed = InitialForkSeed::new(
            session(2),
            session(1),
            source,
            ForkSeed::ExactReplay(exact(session(1))),
            vec![],
            2,
        )
        .unwrap();
        let ForkSeed::ExactReplay(replay) = seed.seed() else {
            panic!("exact seed required")
        };
        assert_eq!(replay.items(), &[item()]);
        assert_eq!(replay.item_origins()[0].original().binding_epoch(), 3);
    }
}

// 배열 끝 첫 초과와 origin 누락을 거부하고 완전한 partition만 허용합니다.
#[test]
fn replay_rejects_missing_origins_and_non_partition_groups() {
    let contract = ModelReplayContract::new("system", vec![]);
    assert!(
        ForkExactReplay::new(
            contract.clone(),
            vec![item()],
            vec![],
            vec![ForkGroup::new(0, 1).unwrap()]
        )
        .is_err()
    );
    assert!(
        ForkExactReplay::new(
            contract.clone(),
            vec![item(), item()],
            vec![origin(session(1), 0, binding(ReplayProfile::SemanticOnly)); 2],
            vec![ForkGroup::new(1, 2).unwrap()]
        )
        .is_err()
    );
    assert!(ForkGroup::new(0, 4097).is_err());
    assert!(ForkItemCoordinate::new(session(1), 0, 1, JournalSequence::new(1), 0).is_err());
    assert!(ForkItemCoordinate::new(session(1), 1, 1, JournalSequence::new(1), 4096).is_err());
    let origins = vec![origin(session(1), 0, binding(ReplayProfile::SemanticOnly)); 4097];
    assert!(
        ForkExactReplay::new(
            contract,
            vec![item(); 4097],
            origins,
            vec![ForkGroup::new(0, 4096).unwrap()]
        )
        .is_err()
    );
}

// private payload의 원래 epoch를 바꾸지 않고 보존하며 의미 전용 binding으로 넘기지 못합니다.
#[test]
fn private_import_preserves_payload_and_rejects_incompatible_binding() {
    let profile = ReplayProfile::ProviderPrivateLocalPlaintext;
    let original_binding = binding(profile);
    let payload = br#"{"reasoning_content":"exact\r\nprivate","content":"visible"}"#.to_vec();
    let envelope = ProviderPrivateReplayEnvelope::new(
        crate::provider_private_schema(profile).unwrap(),
        payload.clone(),
    )
    .unwrap();
    let items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "visible".into(),
            refusal: None,
        },
        ModelReplayItem::ProviderPrivateAssistant { envelope },
    ];
    let replay = ForkExactReplay::new(
        ModelReplayContract::new("system", vec![]),
        items,
        vec![
            origin(session(1), 0, original_binding.clone()),
            origin(session(1), 1, original_binding.clone()),
        ],
        vec![ForkGroup::new(0, 2).unwrap()],
    )
    .unwrap();
    let good = InitialForkSeed::new(
        session(2),
        session(1),
        ForkSource::Anchor(point(9, 8, original_binding)),
        ForkSeed::ExactReplay(replay.clone()),
        vec![],
        2,
    )
    .unwrap();
    let ForkSeed::ExactReplay(exact) = good.seed() else {
        panic!("exact seed required")
    };
    let ModelReplayItem::ProviderPrivateAssistant { envelope } = &exact.items()[1] else {
        panic!("private item required")
    };
    assert_eq!(envelope.payload(), payload);
    assert_eq!(exact.item_origins()[1].original().binding_epoch(), 3);
    assert!(
        InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::Anchor(point(9, 8, binding(ReplayProfile::SemanticOnly))),
            ForkSeed::ExactReplay(replay),
            vec![],
            2
        )
        .is_err()
    );
}

fn history(source: SessionId, sequence: u64) -> ForkHistoryEntry {
    ForkHistoryEntry::new(
        source,
        ForkHistoryCoordinate::Journal {
            sequence: JournalSequence::new(sequence),
        },
        SequencedJournalRecord::with_journal_sequence(
            ReplaySequence::new(sequence),
            JournalSequence::new(sequence),
            JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: source }),
        ),
    )
    .unwrap()
}

// inherited history는 중복 source 좌표, 부모 cutoff 초과, child identity 오염을 거부합니다.
#[test]
fn history_rejects_duplicate_coordinates_future_records_and_size_excess() {
    for entries in [
        vec![history(session(1), 1), history(session(1), 1)],
        vec![history(session(1), 9)],
        vec![history(session(2), 1)],
        vec![history(session(1), 2), history(session(1), 1)],
    ] {
        assert!(
            InitialForkSeed::new(
                session(2),
                session(1),
                ForkSource::Anchor(point(9, 8, binding(ReplayProfile::SemanticOnly))),
                ForkSeed::ExactReplay(exact(session(1))),
                entries,
                128
            )
            .is_err()
        );
    }
    assert!(
        InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::Anchor(point(9, 8, binding(ReplayProfile::SemanticOnly))),
            ForkSeed::ExactReplay(exact(session(1))),
            vec![],
            16 * 1024 * 1024 + 1
        )
        .is_err()
    );
}

// native locator 재사용은 거부하고 schema 문법 검증을 실행 권한 증명으로 취급하지 않습니다.
#[test]
fn native_seed_rejects_source_locator_reuse() {
    let native = BackendBindingEvidence::new(
        "native",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "same"),
        ContinuationStrategy::BackendManagedState,
    );
    assert!(
        InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::Anchor(point(9, 8, native.clone())),
            ForkSeed::BackendNative {
                source_boundary_evidence: VersionedIdentity::new("boundary/v1", "proof"),
                candidate_binding: native
            },
            vec![],
            2
        )
        .is_err()
    );
}

// 정확한 source 경계 증명을 별도로 요구하되 exact source에서 native candidate로의 형식 전환은 막지
// 않습니다.
#[test]
fn native_candidate_can_preserve_host_and_model_across_source_strategy() {
    let source = binding(ReplayProfile::SemanticOnly);
    let candidate = |model: &str| {
        BackendBindingEvidence::new(
            source.backend_kind(),
            source.backend_version(),
            source.binding_identity().clone(),
            BackendIdentity::new("model/v1", model),
            BackendIdentity::new("locator/v1", "child"),
            ContinuationStrategy::BackendManagedState,
        )
    };
    for (model, accepted) in [("model", true), ("changed", false)] {
        let result = InitialForkSeed::new(
            session(2),
            session(1),
            ForkSource::InitialFork(point(6, 8, source.clone())),
            ForkSeed::BackendNative {
                source_boundary_evidence: VersionedIdentity::new("boundary/v1", "proof"),
                candidate_binding: candidate(model),
            },
            vec![],
            2,
        );
        assert_eq!(result.is_ok(), accepted);
    }
}
