use yo_core::{
    CompleteModelBinding, ImageSummarySource, InputImageSnapshot, ModelInputPart, ModelReplayItem,
    RequestToolExposure,
};

use super::*;

fn complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../yo-core/src/model_service/tests/openrouter-binding.json"
    )))
    .unwrap()
}
fn snapshot() -> InputImageSnapshot {
    serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap()
}
fn parts() -> Vec<ModelInputPart> {
    vec![
        ModelInputPart::Image {
            snapshot: snapshot(),
        },
        ModelInputPart::Text {
            text: "한글 between".into(),
        },
        ModelInputPart::Image {
            snapshot: snapshot(),
        },
    ]
}
fn request(input: Vec<ModelConnectorInputItem>) -> ModelConnectorRequest {
    ModelConnectorRequest::new(input, RequestToolExposure::disabled(), 2048, None).unwrap()
}
fn policy() -> ImageWirePolicy {
    ImageWirePolicy::admit(&complete()).unwrap().unwrap()
}

// transport는 반복 PNG 원본과 순서를 지키고 tokenizer에서는 base64 전체를 제외한다.
#[test]
fn ordered_png_and_text_projection_keep_free_routing() {
    let input = request(vec![ModelConnectorInputItem::MultimodalUser {
        parts: parts(),
    }]);
    assert!(wire_body(&input, "model").is_err());
    let body = projected_body(&input, "model", Some(&policy()), false).unwrap();
    let content = &body["messages"][0]["content"];
    let url = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(snapshot().png())
    );
    assert_eq!(content[0]["image_url"]["url"], url);
    assert_eq!(content[1]["text"], "한글 between");
    assert_eq!(content[2]["image_url"]["url"], url);
    assert_eq!(
        input.input_images().map(|s| s.png()).collect::<Vec<_>>(),
        vec![snapshot().png(), snapshot().png()]
    );
    let tokens = projected_body(&input, "model", Some(&policy()), true).unwrap();
    assert_eq!(
        tokens["messages"][0]["content"],
        json!([{"type":"text","text":"한글 between"}])
    );
    assert!(!tokens.to_string().contains("base64"));
    assert_eq!(tokens["provider"], body["provider"]);
    assert_eq!(
        body["provider"]["max_price"],
        json!({"prompt":0,"completion":0,"request":0,"image":0})
    );
    assert_eq!(body["provider"]["allow_fallbacks"], false);
    assert_eq!(body["provider"]["only"], json!(["nvidia"]));
    assert!(body.get("tools").is_none());
}

// 이미지 없는 검증 요청과 재개·tool-result·summary 요청도 같은 routing 제한을 보존한다.
#[test]
fn text_history_tool_result_and_summary_share_the_admitted_route() {
    let policy = policy();
    let user = ModelConnectorInputItem::Message {
        role: ModelConnectorInputRole::User,
        content: "next".into(),
        refusal: None,
    };
    let replay = vec![
        ModelConnectorInputItem::MultimodalUser { parts: parts() },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::Assistant,
            content: "visible".into(),
            refusal: Some("refusal".into()),
        },
        ModelConnectorInputItem::FunctionCall {
            call_id: "c".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        },
        ModelConnectorInputItem::FunctionCallOutput {
            call_id: "c".into(),
            output: "result".into(),
        },
        user.clone(),
    ];
    let body = projected_body(&request(replay), "model", Some(&policy), false).unwrap();
    assert_eq!(body["messages"][1]["refusal"], "refusal");
    assert_eq!(body["messages"][2]["tool_call_id"], "c");
    let text = projected_body(&request(vec![user]), "model", Some(&policy), false).unwrap();
    assert_eq!(body["provider"], text["provider"]);
    let groups = vec![vec![ModelReplayItem::MultimodalUser { parts: parts() }]];
    let source = ImageSummarySource::from_replay_groups(&groups).unwrap();
    let summary = request(vec![ModelConnectorInputItem::ImageSummarySource {
        source: source.clone(),
    }]);
    let body = projected_body(&summary, "model", Some(&policy), false).unwrap();
    assert_eq!(body["provider"], text["provider"]);
    assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 3);
    let manifest: Value =
        serde_json::from_str(body["messages"][0]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(manifest["schema"], "yo.image-summary-source/v1");
    assert_eq!(summary.image_count(), 2);
    assert!(
        !projected_body(&summary, "model", Some(&policy), true)
            .unwrap()
            .to_string()
            .contains("data:image")
    );
}

// 알려진 output cap과 no-tools 정책을 직접 Connector 경계에서도 강제한다.
#[test]
fn rejects_missing_or_excess_cap_and_enabled_tools_before_dispatch() {
    let input = vec![ModelConnectorInputItem::MultimodalUser { parts: parts() }];
    for cap in [None, Some(65537)] {
        let req =
            ModelConnectorRequest::new(input.clone(), RequestToolExposure::disabled(), cap, None)
                .unwrap();
        assert!(projected_body(&req, "model", Some(&policy()), false).is_err());
    }
    let tool = yo_core::FunctionTool::new("read_file", "read", json!({"type":"object"})).unwrap();
    let req =
        ModelConnectorRequest::new(input, RequestToolExposure::enabled(vec![tool]), 2048, None)
            .unwrap();
    assert!(projected_body(&req, "model", Some(&policy()), false).is_err());
}

// summary 경계의 64개 PNG는 전송하고 첫 초과 occurrence는 source 생성에서 거절한다.
#[test]
fn summary_occurrence_boundary_is_not_truncated() {
    let group = vec![ModelReplayItem::MultimodalUser {
        parts: vec![
            ModelInputPart::Image {
                snapshot: snapshot()
            };
            16
        ],
    }];
    let mut groups = vec![group.clone(); 4];
    let source = ImageSummarySource::from_replay_groups(&groups).unwrap();
    let req = request(vec![ModelConnectorInputItem::ImageSummarySource { source }]);
    let body = projected_body(&req, "model", Some(&policy()), false).unwrap();
    assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 65);
    groups.push(vec![ModelReplayItem::MultimodalUser {
        parts: vec![ModelInputPart::Image {
            snapshot: snapshot(),
        }],
    }]);
    assert!(ImageSummarySource::from_replay_groups(&groups).is_err());
}
