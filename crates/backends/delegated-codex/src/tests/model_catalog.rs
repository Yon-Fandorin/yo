use std::time::Duration;

use serde_json::json;

use super::{
    super::{client::AppServerClient, observe_model_catalog},
    support::{FakePeer, initialize_response},
};

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
