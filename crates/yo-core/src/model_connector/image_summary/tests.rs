use serde_json::{Value, json};

use super::{ImageSummarySource, MAX_SOURCE_BYTES, Source};
use crate::{InputImageSnapshot, ModelInputPart, ModelReplayItem, ProviderPrivateReplayEnvelope};

fn snapshot() -> InputImageSnapshot {
    serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap()
}

fn user(text: &str) -> ModelReplayItem {
    ModelReplayItem::MultimodalUser {
        parts: vec![
            ModelInputPart::Text {
                text: text.to_owned(),
            },
            ModelInputPart::Image {
                snapshot: snapshot(),
            },
        ],
    }
}

// 같은 PNG도 원래 등장 순서대로 별도 index를 받고 private payload는 요약 요청에 들어가지 않는다.
#[test]
fn manifest_and_typed_images_preserve_occurrence_order() {
    let source = ImageSummarySource::from_replay_groups(&[vec![
        user("first"),
        ModelReplayItem::ProviderPrivateAssistant {
            envelope: ProviderPrivateReplayEnvelope::new(
                "test/v1",
                br#"{"private":"omit-me"}"#.to_vec(),
            )
            .unwrap(),
        },
        user("second"),
    ]])
    .unwrap();
    assert_eq!(source.parts().len(), 3);
    let ModelInputPart::Text { text } = &source.parts()[0] else {
        panic!("missing manifest")
    };
    assert!(text.starts_with("{\"schema\":\"yo.image-summary-source/v1\",\"history\":"));
    assert!(!text.contains("omit-me"));
    let manifest: Value = serde_json::from_str(text).unwrap();
    assert_eq!(manifest["history"].as_array().unwrap().len(), 2);
    for index in 0..2 {
        assert_eq!(
            manifest["history"][index]["parts"][1],
            json!({
                "type":"image_ref", "index":index, "sha256":snapshot().sha256(),
                "byte_length":70, "width":1, "height":1,
            })
        );
        assert_eq!(
            source.parts()[index + 1],
            ModelInputPart::Image {
                snapshot: snapshot()
            }
        );
    }
}

// 요약 전용 한도는 원래 입력 16개 한도와 독립적이지만 65번째 occurrence는 거절한다.
#[test]
fn summary_accepts_sixty_four_images_and_rejects_first_excess() {
    let mut groups = vec![vec![user("a")]; 64];
    assert_eq!(
        ImageSummarySource::from_replay_groups(&groups)
            .unwrap()
            .parts()
            .len(),
        65
    );
    groups.push(vec![user("a")]);
    assert!(ImageSummarySource::from_replay_groups(&groups).is_err());
}

// manifest의 이중 JSON escape 및 PNG framing까지 포함한 정확한 16 MiB 경계를 검사한다.
#[test]
fn complete_source_budget_counts_escaped_manifest_and_snapshots() {
    let source = ImageSummarySource::from_replay_groups(&[vec![user("a")]]).unwrap();
    let base = serde_json::to_vec(&Source {
        role: "user",
        parts: source.parts(),
    })
    .unwrap()
    .len();
    let text = "a".repeat(1 + MAX_SOURCE_BYTES - base);
    let exact = ImageSummarySource::from_replay_groups(&[vec![user(&text)]]).unwrap();
    assert_eq!(
        serde_json::to_vec(&Source {
            role: "user",
            parts: exact.parts()
        })
        .unwrap()
        .len(),
        MAX_SOURCE_BYTES
    );
    assert!(ImageSummarySource::from_replay_groups(&[vec![user(&(text + "a"))]]).is_err());
}
