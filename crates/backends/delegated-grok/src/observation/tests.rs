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
        protocol::server_error(json!(3), -32601, "Method not found"),
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
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1]["method"], "authenticate");
    assert_eq!(sent[2]["method"], "_x.ai/billing");
    assert_eq!(sent[2]["params"], json!({}));
    assert!(sent.iter().all(|message| {
        !matches!(
            message["method"].as_str(),
            Some("session/new" | "session/load" | "session/prompt")
        )
    }));
}

fn account_messages() -> Vec<Value> {
    vec![
        initialize_response(1, &["cached_token"], true),
        response(
            2,
            json!({"_meta": {
                "email": "person@example.test", "subscription_tier": "supergrok"
            }}),
        ),
    ]
}

fn billing_config(used_percent: f64) -> Value {
    json!({
        "creditUsagePercent": used_percent,
        "currentPeriod": {
            "type": "USAGE_PERIOD_TYPE_WEEKLY",
            "end": "2999-09-01T14:45:00Z"
        }
    })
}

struct UsageLog(std::path::PathBuf);

impl UsageLog {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yo-grok-account-billing-{}.jsonl",
            uuid::Uuid::now_v7()
        ));
        let event = json!({
            "msg": "billing: fetched credits config",
            "ctx": {"config": billing_config(12.1)}
        });
        std::fs::write(&path, format!("{event}\n")).unwrap();
        Self(path)
    }
}

impl Drop for UsageLog {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// 사용 가능한 billing extension의 실제 잔여율·리셋 시각을 snapshot에 전달하고,
// 오래된 로컬 log 값 대신 live 값을 사용하며 Session이나 prompt를 만들지 않습니다.
#[test]
fn reads_live_billing_capacity_instead_of_the_legacy_log() {
    let log = UsageLog::new();
    let mut messages = account_messages();
    messages.push(response(3, json!({"config": billing_config(51.0)})));
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client, Some(&log.0)).unwrap();

    assert_eq!(snapshot.account_label(), "person@example.test");
    assert_eq!(snapshot.buckets()[0].plan(), Some("supergrok"));
    let window = snapshot.buckets()[0].primary().unwrap();
    assert_eq!(window.remaining_percent_basis_points(), 4_900);
    assert_eq!(window.window_duration_minutes(), Some(10_080));
    assert_eq!(
        window.resets_at_unix_seconds(),
        Some(
            "2999-09-01T14:45:00Z"
                .parse::<jiff::Timestamp>()
                .unwrap()
                .as_second()
        )
    );
    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2]["method"], "_x.ai/billing");
    assert_eq!(sent[2]["params"], json!({}));
}

// 구버전 호스트가 정확한 method-not-found를 반환할 때만 bounded log의 기존
// 주간 사용량 경로를 유지합니다.
#[test]
fn uses_the_legacy_log_only_when_the_billing_extension_is_unsupported() {
    let log = UsageLog::new();
    let mut messages = account_messages();
    messages.push(protocol::server_error(json!(3), -32601, "Method not found"));
    let (peer, _) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client, Some(&log.0)).unwrap();

    assert_eq!(
        snapshot.buckets()[0]
            .primary()
            .unwrap()
            .remaining_percent_basis_points(),
        8_790
    );
}

// 지원되는 billing 요청의 실패·잘못된 응답·다른 request의 method-not-found는
// 오래된 log로 성공 처리하지 않고 refresh 실패로 남깁니다.
#[test]
fn rejects_billing_failures_and_mismatched_responses_without_using_the_log() {
    let log = UsageLog::new();
    for reply in [
        protocol::server_error(json!(3), -32000, "Authentication required"),
        protocol::server_error(json!(3), -32603, "Billing service error"),
        protocol::server_error(json!(4), -32601, "Method not found"),
        response(
            3,
            json!({"config": {"creditUsagePercent": "51", "currentPeriod": {
                "type": "USAGE_PERIOD_TYPE_WEEKLY", "end": "2999-09-01T14:45:00Z"
            }}}),
        ),
        response(3, json!({"config": []})),
        response(3, json!({"config": {"currentPeriod": []}})),
        response(3, json!({"config": {"currentPeriod": {"type": 7}}})),
        response(
            3,
            json!({"config": {"currentPeriod": {
                "type": "USAGE_PERIOD_TYPE_WEEKLY", "end": 7
            }}}),
        ),
        response(3, json!({"config": billing_config(100.01)})),
        response(3, json!([])),
    ] {
        let mut messages = account_messages();
        messages.push(reply);
        let (peer, _) = FakePeer::new(messages);
        let mut client = AcpClient::new(peer, Duration::from_secs(1));

        assert!(observe_account_capacity(&mut client, Some(&log.0)).is_err());
    }
}

// 구버전 log의 최신 period가 잘못된 형식이어도 이전 정상 관측값을 계속 탐색해,
// billing extension이 없는 계정의 사용량이 사라지지 않도록 보호합니다.
#[test]
fn skips_malformed_legacy_periods_and_preserves_the_previous_valid_capacity() {
    use std::io::Write;

    let log = UsageLog::new();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&log.0)
        .unwrap();
    for period in [
        json!([]),
        json!({"type": 7, "end": "2999-09-01T14:45:00Z"}),
        json!({"type": "USAGE_PERIOD_TYPE_WEEKLY", "end": 7}),
    ] {
        let event = json!({
            "msg": "billing: fetched credits config",
            "ctx": {"config": {"currentPeriod": period}}
        });
        writeln!(file, "{event}").unwrap();
    }
    drop(file);
    let mut messages = account_messages();
    messages.push(protocol::server_error(json!(3), -32601, "Method not found"));
    let (peer, _) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client, Some(&log.0)).unwrap();

    assert_eq!(
        snapshot.buckets()[0]
            .primary()
            .unwrap()
            .remaining_percent_basis_points(),
        8_790
    );
}

// 현재 billing 응답에 용량이 없으면 오래된 log를 섞거나 0%를 만들지 않고
// 인증된 요금제만 표시합니다.
#[test]
fn keeps_missing_live_capacity_without_substituting_the_legacy_log() {
    let log = UsageLog::new();
    let mut messages = account_messages();
    messages.push(response(3, json!({"config": null})));
    let (peer, _) = FakePeer::new(messages);
    let mut client = AcpClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client, Some(&log.0)).unwrap();

    assert_eq!(snapshot.buckets()[0].plan(), Some("supergrok"));
    assert!(snapshot.buckets()[0].primary().is_none());
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
    assert_eq!(sent.0.borrow().len(), 2);
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
