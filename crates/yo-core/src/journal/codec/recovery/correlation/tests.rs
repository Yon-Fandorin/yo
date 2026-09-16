use super::{
    super::super::{ContextImageLoss, ContextImageSource},
    CorrelationRecovery,
    model::ReplayGroup,
};
use crate::{
    BackendBindingEvidence, BackendIdentity, ContinuationStrategy, JournalSequence, ModelReplay,
    ModelReplayContract, ModelReplayItem, ModelReplayRole, ReplayExecutor, ReplayProfile, TurnId,
};

// source range 검증은 숫자 공간을 순회하지 않고 실제 Journal 좌표만 조회해야 하므로
// 극단적으로 먼 두 sequence도 bounded record 수에 비례해 처리됩니다.
#[test]
fn source_range_validation_is_bounded_by_present_records() {
    let first = JournalSequence::new(1);
    let last = JournalSequence::new(u64::MAX - 1);
    let mut recovery = CorrelationRecovery::default();
    recovery.record_coordinates.insert(first, (7, 9));
    recovery.record_coordinates.insert(last, (7, 9));

    recovery
        .validate_source_range(first, last, (7, 9), "sparse source range")
        .unwrap();
}

// 같은 본문이 두 번 있어도 value로 provenance를 합치지 않고 각 delta 좌표를 보존한다.
// 진행 중 turn이나 하나라도 빠진 origin은 완전한 fork root로 노출하지 않는다.
#[test]
fn fork_replay_keeps_distinct_origins_for_equal_items_and_rejects_partial_state() {
    use crate::{ModelReplayContract, ModelReplayRole, ReplayExecutor, fixture_session};
    let session = fixture_session(1);
    let binding = BackendBindingEvidence::new(
        "managed",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "session"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: ReplayProfile::SemanticOnly,
        },
    );
    let item = ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "same".into(),
        refusal: None,
    };
    let mut recovery = CorrelationRecovery {
        session_id: Some(session),
        open_binding: Some(binding),
        model_replay: ModelReplay::from_checkpoint(
            ModelReplayContract::new("system", vec![]),
            vec![item.clone(), item.clone()],
        )
        .unwrap(),
        ..CorrelationRecovery::default()
    };
    for sequence in [7, 14] {
        let sequence = JournalSequence::new(sequence);
        recovery
            .model_replay_origins
            .push(recovery.local_item_origin(3, 2, sequence, 0).unwrap());
        recovery.replay_groups.push(ReplayGroup {
            first_sequence: sequence,
            last_sequence: sequence,
            replay_delta_sequence: sequence,
            epoch: 3,
            context_epoch: 2,
            items: vec![item.clone()],
            fork_group_index: None,
            image_losses: Vec::new(),
        });
    }
    let replay = recovery.fork_replay().unwrap();
    assert_eq!(
        replay.item_origins()[0].original().record_sequence(),
        JournalSequence::new(7)
    );
    assert_eq!(
        replay.item_origins()[1].original().record_sequence(),
        JournalSequence::new(14)
    );
    assert_eq!(replay.item_origins()[0].original().binding_epoch(), 3);
    recovery
        .active_turn_starts
        .insert(TurnId::new(1.try_into().unwrap()), JournalSequence::new(20));
    assert!(
        recovery
            .fork_replay()
            .unwrap_err()
            .to_string()
            .contains("idle")
    );
    recovery.active_turn_starts.clear();
    recovery.model_replay_origins.pop();
    assert!(recovery.fork_replay().is_err());
}

// binding 교체는 retained checkpoint·fork의 local source 좌표와 원래 context epoch를 바꾸지
// 않는다.
#[test]
fn image_source_coordinates_survive_exact_binding_transfer() {
    let snapshot: crate::InputImageSnapshot = serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap();
    let items = vec![ModelReplayItem::MultimodalUser {
        parts: vec![
            crate::ModelInputPart::Text {
                text: "before".into(),
            },
            crate::ModelInputPart::Image { snapshot },
        ],
    }];
    for imported in [false, true] {
        let losses = ContextImageLoss::for_items(
            &items,
            if imported { 1 } else { 2 },
            |item_index, part_index| {
                if imported {
                    ContextImageSource::InitialForkSeed {
                        sequence: 2,
                        group_index: 0,
                        item_index,
                        part_index,
                    }
                } else {
                    ContextImageSource::RetainedCheckpoint {
                        sequence: 11,
                        group_index: 3,
                        item_index,
                        part_index,
                    }
                }
            },
        )
        .unwrap();
        let mut recovery = CorrelationRecovery {
            context_epoch: Some(4),
            ..Default::default()
        };
        recovery.replay_groups.push(ReplayGroup {
            first_sequence: JournalSequence::new(11),
            last_sequence: JournalSequence::new(11),
            replay_delta_sequence: JournalSequence::new(11),
            epoch: 1,
            context_epoch: 4,
            items: items.clone(),
            fork_group_index: imported.then_some(0),
            image_losses: losses.clone(),
        });
        recovery.rebind_fork_groups(JournalSequence::new(20), 2);
        assert_eq!(recovery.replay_groups[0].image_losses, losses);
        assert_eq!(recovery.replay_groups[0].epoch, 2);
        assert_eq!(recovery.replay_groups[0].context_epoch, 4);
    }
}
