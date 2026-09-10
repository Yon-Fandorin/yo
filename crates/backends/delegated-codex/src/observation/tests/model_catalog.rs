use std::time::Duration;

use serde_json::json;

use super::super::{AppServerClient, observe_model_capability, observe_model_catalog};
use crate::test_support::{FakePeer, initialize_response};

// account/read와 모든 model/list page만 읽어 exact account/model inventory를 만들며
// Agent thread는 생성하지 않습니다.
#[test]
fn reads_paginated_authenticated_model_catalog_without_starting_a_thread() {
    let messages = [
        initialize_response(1, "0.149.1"),
        json!({
            "id": 2,
            "result": {"account": {"type": "chatgpt", "email": "person@example.test", "planType": "pro"}}
        }),
        json!({
            "id": 3,
            "result": {
                "data": [{"model": "gpt-5.6-codex", "displayName": "GPT-5.6 Codex", "isDefault": true, "hidden": false}],
                "nextCursor": "next"
            }
        }),
        json!({
            "id": 4,
            "result": {
                "data": [{"model": "gpt-5.5-codex", "displayName": "GPT-5.5 Codex", "isDefault": false, "hidden": false}],
                "nextCursor": null
            }
        }),
    ];
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let catalog = observe_model_catalog(&mut client).unwrap();

    let controller = yo_core::ModelSelectionController::new(
        yo_core::ModelCatalog::new(Vec::new()).unwrap(),
        None,
    )
    .with_host_catalog(catalog, true);
    let section = &controller.sections()[0];
    assert_eq!(section.label(), "Codex · person@example.test");
    assert_eq!(section.choices()[0].label(), "GPT-5.6 Codex (current)");
    assert_eq!(section.choices()[1].label(), "GPT-5.5 Codex");

    let sent = sent.0.borrow();
    assert_eq!(sent[2]["method"], "account/read");
    assert_eq!(sent[3]["method"], "model/list");
    assert_eq!(sent[4]["params"]["cursor"], "next");
    assert!(
        sent.iter()
            .all(|message| message["method"] != "thread/start")
    );
}

// exact selected model의 explicit modality만 capability로 사용하며 default model의 image
// 선언이나 호환 major warning은 다른 model의 입력 권한을 대신하지 않습니다.
#[test]
fn observes_image_capability_for_the_exact_selected_model() {
    let (peer, _) = FakePeer::new([json!({
        "id": 1,
        "result": {
            "data": [
                {"model": "default-image", "isDefault": true, "inputModalities": ["image"]},
                {"model": "selected-text", "isDefault": false, "inputModalities": ["text"]}
            ],
            "nextCursor": null
        }
    })]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    assert_eq!(
        observe_model_capability(&mut client, "selected-text", true),
        yo_core::ImageInputCapability::Unsupported
    );
}

// hidden row도 picker visibility와 별개로 exact selected model capability의 근거가 되며,
// 같은 ID가 visible/hidden으로 중복되면 complete observation을 Unknown으로 닫습니다.
#[test]
fn observes_hidden_selected_model_and_rejects_visibility_conflicts() {
    let (peer, sent) = FakePeer::new([json!({
        "id": 1,
        "result": {
            "data": [{
                "model": "default-text",
                "isDefault": true,
                "inputModalities": ["text"]
            }, {
                "model": "hidden-image",
                "hidden": true,
                "inputModalities": ["text", "image"]
            }],
            "nextCursor": null
        }
    })]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    assert_eq!(
        observe_model_capability(&mut client, "hidden-image", true),
        yo_core::ImageInputCapability::Supported {
            maximum_occurrences: 16,
            maximum_image_bytes: yo_core::InputImageSnapshot::MAX_BYTES as u64,
            maximum_input_bytes: yo_core::InputImageSnapshot::MAX_BYTES as u64,
        }
    );
    assert_eq!(sent.0.borrow()[0]["params"]["includeHidden"], true);

    let (peer, _) = FakePeer::new([json!({
        "id": 1,
        "result": {
            "data": [
                {"model": "hidden-image", "hidden": true, "inputModalities": ["image"]},
                {"model": "hidden-image", "hidden": false, "inputModalities": ["image"]}
            ],
            "nextCursor": null
        }
    })]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    assert_eq!(
        observe_model_capability(&mut client, "hidden-image", true),
        yo_core::ImageInputCapability::Unknown
    );
}

// modality가 없거나 invalid하면 catalog가 읽혀도 image capability는 Unknown으로 남기고,
// image wire patch가 검토되지 않은 동안에는 명시적 image 선언도 Supported로 승격하지 않습니다.
#[test]
fn keeps_missing_invalid_and_unreviewed_image_evidence_unknown() {
    for modality in [None, Some(json!(null)), Some(json!(["text", "audio"]))] {
        let mut model = json!({"model": "selected"});
        if let Some(modality) = modality {
            model["inputModalities"] = modality;
        }
        let (peer, _) = FakePeer::new([json!({
            "id": 1,
            "result": {"data": [model], "nextCursor": null}
        })]);
        let mut client = AppServerClient::new(peer, Duration::from_secs(1));
        assert_eq!(
            observe_model_capability(&mut client, "selected", true),
            yo_core::ImageInputCapability::Unknown
        );
    }

    let (peer, _) = FakePeer::new([json!({
        "id": 1,
        "result": {
            "data": [{"model": "selected", "inputModalities": ["image"]}],
            "nextCursor": null
        }
    })]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    assert_eq!(
        observe_model_capability(&mut client, "selected", false),
        yo_core::ImageInputCapability::Unknown
    );
}

// model/list가 한 page의 물리 bound를 넘으면 capability를 추정하지 않고 Unknown으로 닫습니다.
#[test]
fn rejects_a_model_list_page_overflow_before_capability_publication() {
    let data = (0..=100)
        .map(|index| json!({"model": format!("model-{index}")}))
        .collect::<Vec<_>>();
    let (peer, _) = FakePeer::new([json!({
        "id": 1,
        "result": {"data": data, "nextCursor": null}
    })]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    assert_eq!(
        observe_model_capability(&mut client, "model-0", true),
        yo_core::ImageInputCapability::Unknown
    );
}

// 반복 cursor는 무한 pagination을 허용하지 않고, 앞선 complete catalog를 capability 근거로
// publish하지 않습니다.
#[test]
fn rejects_a_repeated_model_list_cursor() {
    let page = json!({
        "data": [{"model": "selected", "inputModalities": ["image"]}],
        "nextCursor": "same"
    });
    let (peer, sent) = FakePeer::new([
        json!({"id": 1, "result": page.clone()}),
        json!({"id": 2, "result": page}),
    ]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    assert_eq!(
        observe_model_capability(&mut client, "selected", true),
        yo_core::ImageInputCapability::Unknown
    );
    assert_eq!(sent.0.borrow()[1]["params"]["cursor"], "same");
}
