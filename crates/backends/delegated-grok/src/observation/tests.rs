use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{AccountId, BackendFailure};

use super::{AcpClient, observe_account_capacity, observe_model_catalog, protocol};
use crate::transport::PeerPoll;

#[derive(Clone)]
struct Sent(Rc<RefCell<Vec<Value>>>);

struct FakePeer {
    incoming: VecDeque<Result<PeerPoll, BackendFailure>>,
    sent: Sent,
}

impl FakePeer {
    fn new(messages: impl IntoIterator<Item = Value>) -> (Self, Sent) {
        let sent = Sent(Rc::new(RefCell::new(Vec::new())));
        (
            Self {
                incoming: messages
                    .into_iter()
                    .map(|message| Ok(PeerPoll::Message(message)))
                    .collect(),
                sent: sent.clone(),
            },
            sent,
        )
    }
}

impl JsonMessagePeer for FakePeer {
    fn stop_handle(&self) -> yo_core::BackendStopHandle {
        yo_core::BackendStopHandle::no_op()
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        self.sent.0.borrow_mut().push(message.clone());
        Ok(())
    }

    fn receive(&mut self, _timeout: Duration) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Closed))
    }

    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Pending))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

fn initialize_response(id: u64, auth_methods: &[&str], load_session: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": 1,
            "agentCapabilities": { "loadSession": load_session },
            "authMethods": auth_methods
                .iter()
                .map(|method| json!({ "id": method, "name": method }))
                .collect::<Vec<_>>(),
            "agentInfo": { "name": "grok", "version": "1.0.5" }
        }
    })
}

fn response(id: u64, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

// 계정 조회는 기존 initialize와 cached-token authenticate만 수행하고 Agent Session이나
// prompt를 만들지 않은 채 필수 email과 공개 subscription tier를 공용 snapshot으로 투영합니다.
#[test]
fn reads_account_capacity_from_authentication_metadata_without_a_session() {
    let messages = [
        initialize_response(1, &["cached_token", "grok.com"], true),
        response(
            2,
            json!({
                "_meta": {
                    "email": "ignored@example.test",
                    "auth_mode": "Oidc",
                    "team_id": null,
                    "subscription_tier": "SuperGrok"
                }
            }),
        ),
    ];
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client, None).unwrap();

    assert_eq!(snapshot.provider().as_str(), "grok");
    assert_eq!(snapshot.account_label(), "ignored@example.test");
    assert_ne!(snapshot.account().as_str(), "default");
    assert_eq!(snapshot.buckets().len(), 1);
    assert_eq!(snapshot.buckets()[0].id(), Some("grok"));
    assert_eq!(snapshot.buckets()[0].plan(), Some("SuperGrok"));
    assert!(snapshot.buckets()[0].primary().is_none());
    assert!(snapshot.buckets()[0].secondary().is_none());

    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1]["method"], "authenticate");
    assert!(sent.iter().all(|message| {
        !matches!(
            message["method"].as_str(),
            Some("session/new" | "session/load" | "session/prompt")
        )
    }));
}

// 이메일이 없는 Grok 인증 응답은 capacity cache에 임의의 계정을 만들지 않고 실패해야 합니다.
#[test]
fn rejects_account_capacity_without_an_email_identity() {
    let messages = [
        initialize_response(1, &["cached_token", "grok.com"], true),
        response(2, json!({ "_meta": {"subscription_tier": "SuperGrok"} })),
    ];
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let failure = observe_account_capacity(&mut client, None).unwrap_err();

    assert!(failure.message().contains("no valid `email`"));
    assert!(sent.0.borrow().iter().all(|message| {
        message["method"] != "session/new" && message["method"] != "session/prompt"
    }));
}

// initialize modelState와 authenticate email을 한 snapshot으로 묶고 session/new 없이
// Grok 4.6/4.5 exact model rows를 만듭니다.
#[test]
fn reads_exact_grok_model_catalog_without_creating_a_session() {
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": true},
                "authMethods": [{"id": "cached_token", "name": "cached_token"}],
                "agentInfo": {"name": "grok", "version": "1.0.13"},
                "_meta": {"modelState": {
                    "currentModelId": "grok-4.6",
                    "availableModels": [
                        {"modelId": "grok-4.6", "name": "Grok 4.6"},
                        {"modelId": "grok-4.5", "name": "Grok 4.5"}
                    ]
                }}
            }
        }),
        response(
            2,
            json!({"_meta": {"email": "person@example.test", "subscription_tier": "SuperGrok"}}),
        ),
    ];
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let catalog = observe_model_catalog(&mut client).unwrap();
    let controller = yo_core::ModelSelectionController::new(
        yo_core::ModelCatalog::new(Vec::new()).unwrap(),
        None,
    )
    .with_host_catalog(catalog, true);
    let section = &controller.sections()[0];
    assert_eq!(section.label(), "Grok · person@example.test");
    assert_eq!(section.choices()[0].label(), "Grok 4.6 (current)");
    assert_eq!(section.choices()[1].label(), "Grok 4.5");
    assert!(
        section
            .choices()
            .iter()
            .all(|choice| choice.label() != "Automatic")
    );
    assert!(
        sent.0
            .borrow()
            .iter()
            .all(|message| message["method"] != "session/new")
    );
}

// 플랜을 추측하면 로그인 성공을 용량 정보로 오인하므로 누락·공백·제어문자 tier는
// Unknown으로 꾸미지 않고 protocol 실패로 닫습니다.
#[test]
fn rejects_missing_or_unsafe_account_subscription_tiers() {
    for authentication in [
        json!({ "_meta": {} }),
        json!({ "_meta": { "subscription_tier": null } }),
        json!({ "_meta": { "subscription_tier": "" } }),
        json!({ "_meta": { "subscription_tier": " SuperGrok" } }),
        json!({ "_meta": { "subscription_tier": "Super\nGrok" } }),
    ] {
        assert!(
            protocol::decode_account_capacity(
                authentication,
                None,
                AccountId::new("test").unwrap()
            )
            .is_err()
        );
    }
}
