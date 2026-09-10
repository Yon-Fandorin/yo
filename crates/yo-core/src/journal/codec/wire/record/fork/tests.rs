use serde_json::{Value, json};

use super::{WireForkRecord, WireRecord};
use crate::{
    JournalSequence, fixture_descriptor, fixture_session,
    journal::codec::{
        ForkSeed, ForkSource, InitialForkSeed, JournalCommit, JournalRecord, ReplaySequence,
        SequencedJournalRecord, decode, encode,
    },
};

fn empty_record() -> Value {
    json!({"type":"initial_fork_seed", "journal_sequence":1, "profile":"yo.session-fork-seed/v1",
        "parent_session_id":fixture_session(1).to_string(), "source":{"kind":"empty"},
        "seed":{"kind":"empty"}, "history":[]})
}

fn decode_record(value: Value) -> bool {
    serde_json::from_value::<WireRecord>(value)
        .ok()
        .is_some_and(|r| r.decode_in_session(Some(fixture_session(2))).is_ok())
}

// 같은 commit의 child descriptor를 먼저 읽은 후 empty seed를 실제 wire로 왕복합니다.
#[test]
fn initial_seed_round_trips_in_incremental_commit() {
    let child = fixture_session(2);
    let seed = InitialForkSeed::new(
        child,
        fixture_session(1),
        ForkSource::Empty,
        ForkSeed::Empty,
        vec![],
        2,
    )
    .unwrap();
    let commit = JournalCommit::incremental_through(
        JournalSequence::new(1),
        vec![
            SequencedJournalRecord::storage(
                ReplaySequence::new(1),
                JournalRecord::SessionDescriptor(fixture_descriptor(child)),
            ),
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(2),
                JournalSequence::new(1),
                JournalRecord::InitialForkSeed(Box::new(seed)),
            ),
        ],
    );
    let encoded = encode(&commit).unwrap();
    assert_eq!(decode(&encoded).unwrap(), commit);
    let mut value: Value = serde_json::from_str(&encoded).unwrap();
    value["records"].as_array_mut().unwrap().swap(0, 1);
    assert!(decode(&value.to_string()).is_err());
}

// unknown/null/zero와 source·seed 부가 필드를 저장 경계에서 거부합니다.
#[test]
fn closed_seed_grammar_rejects_unknown_null_and_invalid_identity() {
    assert!(decode_record(empty_record()));
    for field in [
        "profile",
        "parent_session_id",
        "source",
        "seed",
        "history",
        "journal_sequence",
    ] {
        let mut value = empty_record();
        value[field] = Value::Null;
        assert!(!decode_record(value), "{field}");
    }
    for path in [
        vec!["unknown"],
        vec!["source", "unexpected"],
        vec!["seed", "unexpected"],
    ] {
        let mut value = empty_record();
        if path.len() == 1 {
            value[path[0]] = json!(true);
        } else {
            value[path[0]][path[1]] = json!(true);
        }
        assert!(!decode_record(value));
    }
    let mut same_child = empty_record();
    same_child["parent_session_id"] = json!(fixture_session(2).to_string());
    assert!(!decode_record(same_child));
    let mut zero = empty_record();
    zero["journal_sequence"] = json!(0);
    assert!(!decode_record(zero));
    let wire: WireRecord = serde_json::from_value(empty_record()).unwrap();
    assert!(wire.decode_in_session(None).is_err());
}

// nested seed나 backend record는 archive record의 폐쇄된 다섯 종류에 포함되지 않습니다.
#[test]
fn history_cannot_embed_live_backend_or_recursive_seed_records() {
    for kind in [
        "initial_fork_seed",
        "backend_request_accepted",
        "backend_binding_opened",
        "context_checkpoint",
    ] {
        let mut value = empty_record();
        value["history"] = json!([{"source_session_id":fixture_session(1).to_string(),
            "source_coordinate":{"kind":"journal","sequence":1}, "record":{"type":kind}}]);
        assert!(serde_json::from_value::<WireRecord>(value).is_err());
    }
}

// 새 initial_fork transition은 cache 없이 인코딩하며 기존 initial 바이트는 그대로 유지합니다.
#[test]
fn transition_grammar_preserves_legacy_and_closes_fork_variants() {
    use super::super::super::correlation::WireBindingTransition;
    use crate::journal::codec::{BindingTransition, CacheState, TransitionMode};
    let initial = BindingTransition::new(TransitionMode::Initial, CacheState::NotApplicable, None);
    assert_eq!(
        serde_json::to_string(&WireBindingTransition::encode(&initial).unwrap()).unwrap(),
        r#"{"mode":"initial","cache":"not_applicable"}"#
    );
    let fork = BindingTransition::initial_fork(JournalSequence::new(1));
    assert_eq!(
        serde_json::to_value(WireBindingTransition::encode(&fork).unwrap()).unwrap(),
        json!({"mode":"initial_fork","fork_seed_sequence":1})
    );
    for value in [
        json!({"mode":"initial_fork","fork_seed_sequence":null}),
        json!({"mode":"initial_fork","fork_seed_sequence":0}),
        json!({"mode":"initial_fork","fork_seed_sequence":1,"cache":"not_applicable"}),
        json!({"mode":"initial_fork","fork_seed_sequence":1,"extra":1}),
        json!({"mode":"exact_replay","cache":"lost","source_initial_fork_sequence":null}),
        json!({"mode":"exact_replay","cache":"lost","source_initial_fork_sequence":0}),
        json!({"mode":"exact_replay","cache":"lost","source_initial_fork_sequence":1,"source_anchor_sequence":2}),
        json!({"mode":"initial","cache":"not_applicable","source_initial_fork_sequence":1}),
    ] {
        assert!(
            serde_json::from_value::<WireBindingTransition>(value)
                .ok()
                .and_then(|v| v.decode().ok())
                .is_none()
        );
    }
    let exact = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_initial_fork_sequence(JournalSequence::new(1));
    let encoded = serde_json::to_string(&WireBindingTransition::encode(&exact).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<WireBindingTransition>(&encoded)
            .unwrap()
            .decode()
            .unwrap(),
        exact
    );
}

// profile이 잘못되면 typed constructor까지 도달해도 실행 가능한 seed로 인정하지 않습니다.
#[test]
fn direct_seed_decode_checks_profile() {
    let mut value = empty_record();
    value.as_object_mut().unwrap().remove("type");
    value["profile"] = json!("future");
    let wire: WireForkRecord = serde_json::from_value(value).unwrap();
    assert!(wire.decode(fixture_session(2)).is_err());
}

// 원문 길이가 아니라 JSON escape를 포함한 byte 수를 세고 정확한 한도 다음 byte부터 거부합니다.
#[test]
fn bounded_counter_counts_encoded_bytes_and_rejects_first_excess() {
    let text = "\n".repeat((super::HISTORY_LIMIT - 2) / 2);
    let mut counter = super::HistoryByteCounter { bytes: 0 };
    serde_json::to_writer(&mut counter, &text).unwrap();
    assert_eq!(counter.bytes, super::HISTORY_LIMIT);
    let mut overflow = super::HistoryByteCounter { bytes: 0 };
    assert!(serde_json::to_writer(&mut overflow, &(text + "x")).is_err());
    assert!(overflow.bytes <= super::HISTORY_LIMIT);
}

// imported private item은 checkpoint epoch로 바뀌지 않고 원래 epoch와 payload bytes를 왕복합니다.
#[test]
fn imported_retained_group_preserves_private_epoch_and_legacy_wire() {
    use super::super::WireContextRetainedGroup;
    use crate::{
        ModelReplayItem, ModelReplayRole, ProviderPrivateReplayEnvelope, ReplayProfile,
        journal::codec::ContextRetainedGroup, provider_private_schema,
    };
    let payload = br#"{"reasoning_content":"original","content":"visible"}"#.to_vec();
    let items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "visible".into(),
            refusal: None,
        },
        ModelReplayItem::ProviderPrivateAssistant {
            envelope: ProviderPrivateReplayEnvelope::new(
                provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
                payload.clone(),
            )
            .unwrap(),
        },
    ];
    let group =
        ContextRetainedGroup::try_imported(JournalSequence::new(2), 4, items.clone(), vec![3])
            .unwrap();
    let encoded = serde_json::to_string(&WireContextRetainedGroup::encode(&group, 1)).unwrap();
    let wire: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(wire["profile"], "yo.fork-retained-group/v1");
    assert_eq!(wire["items"][1]["binding_epoch"], 3);
    assert!(wire.get("private_epochs").is_none());
    assert!(wire.get("first_sequence").is_none());
    let mut invalid_epoch = wire.clone();
    invalid_epoch["items"][1]["binding_epoch"] = json!(0);
    assert!(
        serde_json::from_value::<WireContextRetainedGroup>(invalid_epoch)
            .unwrap()
            .decode(1)
            .is_err()
    );
    let decoded = serde_json::from_str::<WireContextRetainedGroup>(&encoded)
        .unwrap()
        .decode(1)
        .unwrap();
    assert_eq!(decoded, group);
    assert_eq!(decoded.fork_import(), Some((JournalSequence::new(2), 4)));
    assert_eq!(decoded.private_epochs(), &[3]);
    let ModelReplayItem::ProviderPrivateAssistant { envelope } = &decoded.items()[1] else {
        panic!("private item")
    };
    assert_eq!(envelope.payload(), payload);
    for epochs in [vec![], vec![0], vec![3, 3]] {
        assert!(
            ContextRetainedGroup::try_imported(JournalSequence::new(2), 4, items.clone(), epochs)
                .is_err()
        );
    }
    assert!(
        ContextRetainedGroup::try_imported(JournalSequence::new(2), 4096, items, vec![3]).is_err()
    );
    let local = ContextRetainedGroup::try_new(
        JournalSequence::new(5),
        JournalSequence::new(6),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "old".into(),
            refusal: None,
        }],
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&WireContextRetainedGroup::encode(&local, 1)).unwrap(),
        r#"{"first_sequence":5,"last_sequence":6,"items":[{"kind":"message","role":"user","content":"old"}]}"#
    );
    assert!(local.private_epochs().is_empty());
}

// imported group의 새 wire는 필드 혼합·null·잘못된 profile·0 epoch를 모두 거부합니다.
#[test]
fn imported_retained_group_has_closed_wire_fields() {
    use super::super::WireContextRetainedGroup;
    let valid = json!({"profile":"yo.fork-retained-group/v1", "fork_seed_sequence":2, "group_index":0,
        "items":[{"kind":"message","role":"user","content":"one"}]});
    assert!(
        serde_json::from_value::<WireContextRetainedGroup>(valid.clone())
            .unwrap()
            .decode(1)
            .is_ok()
    );
    for (field, value) in [
        ("profile", json!("other")),
        ("profile", Value::Null),
        ("fork_seed_sequence", json!(0)),
        ("fork_seed_sequence", Value::Null),
        ("group_index", json!(4096)),
        ("group_index", Value::Null),
        ("items", Value::Null),
        ("items", json!([])),
        ("first_sequence", json!(2)),
        ("private_epochs", json!([3])),
    ] {
        let mut candidate = valid.clone();
        candidate[field] = value;
        assert!(
            serde_json::from_value::<WireContextRetainedGroup>(candidate)
                .ok()
                .and_then(|v| v.decode(1).ok())
                .is_none(),
            "{field}"
        );
    }
}

// 첫 entry의 JSON escape가 한도를 넘으면 뒤의 잘못된 record를 검사하거나 복제하지 않는다.
// 원문은 한도보다 작아도 encoded history 제한이 먼저 적용되어야 한다.
#[test]
fn seed_factory_stops_at_encoded_excess_before_the_next_invalid_record() {
    use crate::{
        ActivityId, ActivityRef, ActivityUpdate, AgentEvent, TurnId, TurnRef,
        journal::codec::ForkHistoryCoordinate,
    };
    let parent = fixture_session(1);
    let child = fixture_session(2);
    let sequence = JournalSequence::new(1);
    let activity = ActivityRef::new(
        TurnRef::new(parent, TurnId::new(1.try_into().unwrap())),
        ActivityId::new(1.try_into().unwrap()),
    );
    let oversized = SequencedJournalRecord::with_journal_sequence(
        ReplaySequence::new(1),
        sequence,
        JournalRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta("\n".repeat(super::HISTORY_LIMIT / 2)),
        }),
    );
    let invalid = SequencedJournalRecord::with_journal_sequence(
        ReplaySequence::new(2),
        JournalSequence::new(2),
        JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child }),
    );
    let error = super::prepare_seed(
        child,
        parent,
        ForkSource::Empty,
        ForkSeed::Empty,
        vec![
            (
                parent,
                ForkHistoryCoordinate::Journal { sequence },
                &oversized,
            ),
            (
                parent,
                ForkHistoryCoordinate::Journal {
                    sequence: JournalSequence::new(2),
                },
                &invalid,
            ),
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("byte limit"), "{error}");
    let later_error = super::prepare_seed(
        child,
        parent,
        ForkSource::Empty,
        ForkSeed::Empty,
        vec![(
            parent,
            ForkHistoryCoordinate::Journal {
                sequence: JournalSequence::new(2),
            },
            &invalid,
        )],
    )
    .unwrap_err();
    assert!(
        later_error.to_string().contains("qualified Session"),
        "{later_error}"
    );
}

fn image_input() -> crate::UserInput {
    let snapshot: crate::InputImageSnapshot = serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap();
    crate::UserInput::new("[image]")
        .with_images(vec![
            crate::InputImage::new(0..7, 4_194_304, snapshot).unwrap(),
        ])
        .unwrap()
}

// 실제 replay writer는 kind·parts와 snapshot field 순서를 보존하고 closed shape를 강제한다.
#[test]
fn multimodal_replay_wire_keeps_exact_order_and_rejects_malformed_parts() {
    use super::super::{WireModelReplayItem, decode_model_replay_items, encode_model_replay_item};
    let input = image_input();
    let item = input.model_replay_item();
    let bytes = serde_json::to_string(&encode_model_replay_item(&item, 1)).unwrap();
    let snapshot = serde_json::to_string(input.images()[0].snapshot()).unwrap();
    assert_eq!(
        bytes,
        format!(
            r#"{{"kind":"multimodal_user","parts":[{{"type":"image","snapshot":{snapshot}}}]}}"#
        )
    );
    let wire = serde_json::from_str::<WireModelReplayItem>(&bytes).unwrap();
    assert_eq!(
        decode_model_replay_items(vec![wire], 1).unwrap(),
        vec![item]
    );
    for malformed in [
        r#"{"kind":"multimodal_user","parts":null}"#.to_owned(),
        r#"{"kind":"multimodal_user","parts":[]}"#.to_owned(),
        r#"{"kind":"multimodal_user","parts":[{"type":"text","text":"only"}]}"#.to_owned(),
        bytes.replace("\"parts\":[", "\"parts\":[],\"parts\":["),
        bytes.replace(
            "\"parts\":[",
            "\"parts\":[{\"type\":\"text\",\"text\":\"\"},",
        ),
        bytes.replace(
            "\"parts\":[",
            "\"parts\":[{\"type\":\"text\",\"text\":\"a\"},{\"type\":\"text\",\"text\":\"b\"},",
        ),
        bytes.replace("\"snapshot\":", "\"unknown\":true,\"snapshot\":"),
    ] {
        assert!(serde_json::from_str::<WireModelReplayItem>(&malformed).is_err());
    }
}

// 부모의 archived input은 PNG뿐 아니라 원본 source charge도 그대로 물려주므로 재편집 예산이
// 유지된다.
#[test]
fn inherited_image_input_preserves_original_source_evidence() {
    use crate::{
        AgentCommand, SubmissionId, TurnId, TurnRef, journal::codec::ForkHistoryCoordinate,
    };
    let parent = fixture_session(1);
    let child = fixture_session(2);
    let input = image_input();
    let command = AgentCommand::StartTurn {
        turn: TurnRef::new(parent, TurnId::new(1.try_into().unwrap())),
        input: input.clone(),
    };
    let record = SequencedJournalRecord::with_journal_sequence(
        ReplaySequence::new(1),
        JournalSequence::new(1),
        JournalRecord::CommandCommitted(
            crate::journal::CommittedCommand::submission(command, SubmissionId::new().unwrap())
                .unwrap(),
        ),
    );
    let binding = crate::BackendBindingEvidence::new(
        "managed",
        "1",
        crate::BackendIdentity::new("binding/v1", "image-source"),
        crate::BackendIdentity::new("model/v1", "image-model"),
        crate::BackendIdentity::new("locator/v1", "parent"),
        crate::ContinuationStrategy::ExactReplay {
            executor: crate::ReplayExecutor::LocalClient,
            replay_profile: crate::ReplayProfile::SemanticOnly,
        },
    );
    let source = ForkSource::Anchor(
        crate::journal::codec::ForkSourcePoint::new(
            1,
            1,
            JournalSequence::new(3),
            JournalSequence::new(2),
            binding.clone(),
        )
        .unwrap(),
    );
    let coordinate =
        crate::journal::codec::ForkItemCoordinate::new(parent, 1, 1, JournalSequence::new(2), 0)
            .unwrap();
    let replay = crate::journal::codec::ForkExactReplay::new(
        crate::ModelReplayContract::new("system", vec![]),
        vec![input.model_replay_item()],
        vec![crate::journal::codec::ForkItemOrigin::new(coordinate, coordinate, binding).unwrap()],
        vec![crate::journal::codec::ForkGroup::new(0, 1).unwrap()],
    )
    .unwrap();
    let seed = super::prepare_seed(
        child,
        parent,
        source,
        ForkSeed::ExactReplay(replay),
        vec![(
            parent,
            ForkHistoryCoordinate::Journal {
                sequence: JournalSequence::new(1),
            },
            &record,
        )],
    )
    .unwrap();
    let bytes =
        serde_json::to_string(&WireForkRecord::encode(JournalSequence::new(1), &seed).unwrap())
            .unwrap();
    let (_, recovered) = serde_json::from_str::<WireForkRecord>(&bytes)
        .unwrap()
        .decode(child)
        .unwrap();
    let JournalRecord::InitialForkSeed(recovered) = recovered else {
        panic!("initial fork seed");
    };
    assert_eq!(recovered.history(), seed.history());
    let JournalRecord::CommandCommitted(committed) = recovered.history()[0].record().record()
    else {
        panic!("archived input");
    };
    let AgentCommand::StartTurn {
        input: inherited, ..
    } = committed.command()
    else {
        panic!("start turn");
    };
    assert_eq!(inherited, &input);
    assert_eq!(inherited.images()[0].source_byte_length(), 4_194_304);
    let mut malformed: Value = serde_json::from_str(&bytes).unwrap();
    malformed["history"][0]["record"]["command"]["input"]["images"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_byte_length");
    assert!(serde_json::from_value::<WireForkRecord>(malformed).is_err());
}
