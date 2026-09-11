use super::*;
use crate::{
    ContextAccounting, ContextAccountingQuality, InputImageSnapshot, ModelInputPart,
    VersionedProfileId,
    journal::codec::{ContextImageLoss, ContextImageSource, validate_image_losses},
};

fn snapshot() -> InputImageSnapshot {
    serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap()
}

fn accounting(planning: u64, images: bool) -> ContextAccounting {
    let reserve = if images { 1024 } else { 0 };
    ContextAccounting::new(
        ContextAccountingQuality::AdvisoryEstimate,
        VersionedProfileId::new(crate::KIMI_CODE_IMAGE_ACCOUNTING_PROFILE).unwrap(),
        planning - reserve,
        reserve,
    )
    .unwrap()
}

fn binding() -> String {
    let value = serde_json::json!({"provider":"kimi","account":"default","model":"k3-256k","connector":"kimi-chat-completions","base_url":"https://api.kimi.com/coding/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":262144,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{"thinking":{"type":"enabled","keep":"all"}},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1","image_input_profile":"kimi-code-png-advisory/v1"});
    crate::CompleteModelBinding::from_durable_json(&value.to_string()).unwrap();
    value.to_string()
}

fn items(images: bool) -> Vec<ModelReplayItem> {
    let mut items = Vec::new();
    if images {
        items.push(ModelReplayItem::MultimodalUser {
            parts: vec![
                ModelInputPart::Image {
                    snapshot: snapshot(),
                },
                ModelInputPart::Image {
                    snapshot: snapshot(),
                },
            ],
        });
    }
    items.push(ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: "done".into(),
        refusal: None,
    });
    items.push(ModelReplayItem::ProviderPrivateAssistant {
        envelope: ProviderPrivateReplayEnvelope::new(
            crate::provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
            br#"{"content":"done","reasoning_content":"kept"}"#.to_vec(),
        )
        .unwrap(),
    });
    items
}

fn history(images: bool) -> Vec<JournalCommit> {
    current_history_with(ReplayProfile::ProviderPrivateLocalPlaintext, items(images))
        .into_iter()
        .map(|commit| {
            let mut wire: serde_json::Value =
                serde_json::from_str(&encode(&commit).unwrap()).unwrap();
            for record in wire["records"].as_array_mut().unwrap() {
                if record["type"] == "backend_binding_opened" {
                    record["binding_identity"]["value"] = binding().into();
                }
            }
            decode(&wire.to_string()).unwrap()
        })
        .collect()
}

fn image_losses() -> Vec<ContextImageLoss> {
    ContextImageLoss::for_items(&items(true), 1, |item_index, part_index| {
        ContextImageSource::ReplayDelta {
            sequence: 8,
            item_index,
            part_index,
        }
    })
    .unwrap()
}

fn image_checkpoint(images: bool, retain: bool) -> ContextCheckpoint {
    let source = items(images);
    let groups = if retain {
        vec![
            ContextRetainedGroup::try_new(
                JournalSequence::new(7),
                JournalSequence::new(9),
                source.clone(),
            )
            .unwrap(),
        ]
    } else {
        vec![]
    };
    let mut losses = Vec::new();
    if !retain {
        losses.push(
            ContextLoss::visible_prefix_summarized(
                JournalSequence::new(7),
                JournalSequence::new(9),
            )
            .unwrap(),
        );
        let ModelReplayItem::ProviderPrivateAssistant { envelope } = source.last().unwrap() else {
            panic!("private item");
        };
        losses.push(
            ContextLoss::provider_private_dropped(
                envelope.schema(),
                envelope.payload().len() as u64,
                JournalSequence::new(8),
            )
            .unwrap(),
        );
        if images {
            losses.extend(
                image_losses()
                    .into_iter()
                    .map(ContextLoss::ImageInputSummarized),
            );
        }
    }
    let first = groups.first().map(ContextRetainedGroup::first_sequence);
    ContextCheckpoint::try_new(
        1,
        1,
        2,
        JournalSequence::new(10),
        JournalSequence::new(10),
        1,
        ContextStrategy::PortableSummaryV1Alpha1,
        262_144,
        90_000,
        20_000,
        ModelReplayContract::new("system", vec![]),
        portable_body(),
        groups,
        first,
        vec![],
        losses,
        summary_usage(),
    )
    .unwrap()
    .with_accounting(
        accounting(90_000, images),
        accounting(20_000, images && retain),
    )
    .unwrap()
}

fn checkpoint_commit(checkpoint: ContextCheckpoint) -> JournalCommit {
    JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint),
        )],
    )
}

// N=0의 이미지 accounting binding도 v2를 쓰며 v1 scalar checkpoint로 강등하면 복구가 실패한다.
#[test]
fn zero_image_binding_keeps_v2_accounting_and_rejects_legacy_scalars() {
    let checkpoint = image_checkpoint(false, false);
    let commit = checkpoint_commit(checkpoint);
    let bytes = encode(&commit).unwrap();
    assert!(bytes.contains("\"profile\":\"yo.context-checkpoint/v2alpha1\""));
    assert!(!bytes.contains("input_tokens_before"));
    assert!(bytes.contains("\"accounting_before\":{\"quality\":\"advisory_estimate\",\"policy\":\"kimi-code-image-advisory/v1\",\"input_estimate\":90000,\"reserve_tokens\":0}"));
    let mut commits = history(false);
    commits.push(decode(&bytes).unwrap());
    assert!(recover(&commits).is_ok());
    let mut legacy: serde_json::Value = serde_json::from_str(&bytes).unwrap();
    let value = &mut legacy["records"][0];
    value["profile"] = "yo.context-checkpoint/v1alpha1".into();
    value.as_object_mut().unwrap().remove("accounting_before");
    value.as_object_mut().unwrap().remove("accounting_after");
    value["input_tokens_before"] = 90_000.into();
    value["input_tokens_after"] = 20_000.into();
    *commits.last_mut().unwrap() = decode(&legacy.to_string()).unwrap();
    assert!(
        recover(&commits)
            .unwrap_err()
            .to_string()
            .contains("owning binding")
    );
}

// 이미지 손실은 같은 PNG의 반복 occurrence도 각각 정확한 source part 순서로 기록한다.
#[test]
fn image_losses_are_exact_ordered_occurrences_and_cannot_be_forged() {
    let commit = checkpoint_commit(image_checkpoint(true, false));
    let bytes = encode(&commit).unwrap();
    let mut commits = history(true);
    commits.push(decode(&bytes).unwrap());
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.model_replay().items().len(), 1);
    for mutation in [
        "missing",
        "duplicate",
        "swap",
        "wrong-epoch",
        "wrong-part",
        "wrong-hash",
        "wrong-length",
        "wrong-source",
        "null-accounting",
        "legacy-scalar",
    ] {
        let mut wire: serde_json::Value = serde_json::from_str(&bytes).unwrap();
        let value = &mut wire["records"][0];
        match mutation {
            "missing" => {
                value["losses"].as_array_mut().unwrap().pop();
            },
            "duplicate" => value["losses"][3] = value["losses"][2].clone(),
            "swap" => value["losses"].as_array_mut().unwrap().swap(2, 3),
            "wrong-epoch" => value["losses"][2]["source_context_epoch"] = 2.into(),
            "wrong-part" => value["losses"][2]["source"]["part_index"] = 2.into(),
            "wrong-hash" => {
                value["losses"][2]["content_hash"] = format!("sha256:{}", "0".repeat(64)).into()
            },
            "wrong-length" => value["losses"][2]["byte_count"] = 71.into(),
            "wrong-source" => value["losses"][2]["source"]["sequence"] = 7.into(),
            "null-accounting" => value["accounting_before"] = serde_json::Value::Null,
            "legacy-scalar" => value["input_tokens_before"] = 90_000.into(),
            _ => unreachable!(),
        }
        if let Ok(decoded) = decode(&wire.to_string()) {
            *commits.last_mut().unwrap() = decoded;
            assert!(recover(&commits).is_err(), "{mutation}");
        }
    }
}

// retained group에는 정확한 PNG가 남고 표시 observation은 advisory와 image loss를 별도로 노출한다.
#[test]
fn retained_png_bytes_and_advisory_observation_survive_checkpoint_wire() {
    let checkpoint = image_checkpoint(true, true);
    let observation = crate::ContextCheckpointObservation::from(&checkpoint);
    assert_eq!(observation.accounting().unwrap().1.reserve_tokens(), 1024);
    assert_eq!(observation.image_input_loss_count(), 0);
    assert_eq!(observation.provider_private_loss_count(), 0);
    let mut commits = history(true);
    commits.push(decode(&encode(&checkpoint_commit(checkpoint)).unwrap()).unwrap());
    let recovered = recover(&commits).unwrap();
    assert_eq!(&recovered.model_replay().items()[1..], items(true));
    let observation = crate::ContextCheckpointObservation::from(&image_checkpoint(true, false));
    assert_eq!(observation.image_input_loss_count(), 2);
    assert_eq!(observation.provider_private_loss_count(), 1);
}

// 손실 occurrence 64개는 허용하고 65번째와 범위 밖 좌표·치수는 새 checkpoint 전에 거부한다.
#[test]
fn image_loss_bounds_keep_occurrence_identity() {
    let loss = image_losses()[0].clone();
    assert!(validate_image_losses(&vec![loss.clone(); 64]).is_ok());
    assert!(validate_image_losses(&vec![loss; 65]).is_err());
    for source in [
        ContextImageSource::ReplayDelta {
            sequence: 0,
            item_index: 0,
            part_index: 0,
        },
        ContextImageSource::RetainedCheckpoint {
            sequence: 1,
            group_index: 4096,
            item_index: 0,
            part_index: 0,
        },
        ContextImageSource::InitialForkSeed {
            sequence: 1,
            group_index: 0,
            item_index: 0,
            part_index: 33,
        },
    ] {
        assert!(ContextImageLoss::from_snapshot(&snapshot(), 1, source).is_err());
    }
}

// 새 정책의 checkpoint를 journal에서 복원하고 다른 이미지 정책으로 위조하면 epoch 검사가 거절한다.
#[test]
fn openrouter_checkpoint_recovers_only_under_its_own_accounting_policy() {
    let binding = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/model_service/tests/openrouter-binding.json"
    ));
    let semantic_items = items(true)
        .into_iter()
        .filter(|item| !matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }))
        .collect();
    let mut commits = current_history_with(ReplayProfile::SemanticOnly, semantic_items)
        .into_iter()
        .map(|commit| {
            let mut wire: serde_json::Value =
                serde_json::from_str(&encode(&commit).unwrap()).unwrap();
            for record in wire["records"].as_array_mut().unwrap() {
                if record["type"] == "backend_binding_opened" {
                    record["binding_identity"]["value"] = binding.into();
                }
            }
            decode(&wire.to_string()).unwrap()
        })
        .collect::<Vec<_>>();
    let mut wire: serde_json::Value =
        serde_json::from_str(&encode(&checkpoint_commit(image_checkpoint(true, false))).unwrap())
            .unwrap();
    let checkpoint = &mut wire["records"][0];
    checkpoint["input_token_limit"] = 256000.into();
    checkpoint["losses"].as_array_mut().unwrap().remove(1); // no provider-private input on semantic-only replay
    for field in ["accounting_before", "accounting_after"] {
        checkpoint[field]["policy"] = crate::OPENROUTER_FREE_IMAGE_ACCOUNTING_PROFILE.into();
    }
    commits.push(decode(&wire.to_string()).unwrap());
    assert!(recover(&commits).is_ok());
    for field in ["accounting_before", "accounting_after"] {
        wire["records"][0][field]["policy"] = crate::KIMI_CODE_IMAGE_ACCOUNTING_PROFILE.into();
    }
    *commits.last_mut().unwrap() = decode(&wire.to_string()).unwrap();
    assert!(
        recover(&commits)
            .unwrap_err()
            .to_string()
            .contains("owning binding")
    );
}
