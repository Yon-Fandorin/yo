use std::time::Duration;

use serde_json::json;

use super::{
    super::{client::AppServerClient, observe_account_capacity},
    support::{FakePeer, initialize_response},
};

// 계정 조회는 Agent thread를 만들지 않고 initialize 뒤 identity와 rate-limit을 각각 한 번씩
// 읽으며, 다중 limit bucket을 안정된 key 순서와 공용 용량 값으로 변환합니다.
#[test]
fn reads_account_capacity_without_creating_a_session() {
    let account_response = json!({
        "id": 2,
        "result": {
            "account": {
                "id": "acct-1",
                "email": "person@example.test",
                "planType": "plus"
            }
        }
    });
    let rate_limits_response = json!({
        "id": 3,
        "result": {
            "rateLimits": {},
            "rateLimitsByLimitId": {
                "z-extra": {
                    "limitName": "Extra",
                    "primary": { "usedPercent": 5 }
                },
                "codex": {
                    "planType": "plus",
                    "primary": {
                        "usedPercent": 37,
                        "windowDurationMins": 300,
                        "resetsAt": 1800000000
                    },
                    "secondary": { "usedPercent": 71 },
                    "credits": {
                        "balance": "12.5",
                        "hasCredits": true,
                        "unlimited": false
                    }
                }
            }
        }
    });
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.149.1"),
        account_response,
        rate_limits_response,
    ]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client).unwrap();

    assert_eq!(snapshot.provider().as_str(), "codex");
    assert_eq!(snapshot.account_label(), "person@example.test");
    assert_ne!(snapshot.account().as_str(), "default");
    assert_eq!(snapshot.buckets().len(), 2);
    assert_eq!(snapshot.buckets()[0].id(), Some("codex"));
    assert_eq!(snapshot.buckets()[0].plan(), Some("plus"));
    assert_eq!(
        snapshot.buckets()[0].primary().unwrap().remaining_percent(),
        63
    );
    assert_eq!(snapshot.buckets()[1].id(), Some("z-extra"));

    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 4);
    assert_eq!(sent[2]["method"], "account/read");
    assert_eq!(sent[3]["method"], "account/rateLimits/read");
    assert!(sent[3]["params"].is_null());
    assert!(sent.iter().all(|message| {
        !matches!(
            message["method"].as_str(),
            Some("thread/start" | "thread/resume")
        )
    }));
}

// Provider가 계약 범위를 벗어난 사용률을 보내면 잔여량을 음수나 포화값으로 꾸미지 않고
// protocol 실패로 닫아 잘못된 계정 상태가 사용자에게 표시되지 않게 합니다.
#[test]
fn rejects_out_of_range_account_usage() {
    let account_response = json!({
        "id": 2,
        "result": {"account": {"id": "acct-1", "email": "person@example.test"}}
    });
    let response = json!({
        "id": 3,
        "result": {
            "rateLimits": {
                "primary": { "usedPercent": 101 }
            }
        }
    });
    let (peer, _) = FakePeer::new([
        initialize_response(1, "0.149.1"),
        account_response,
        response,
    ]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let failure = observe_account_capacity(&mut client).unwrap_err();

    assert!(failure.message().contains("between 0 and 100"));
}

// 이메일이 없는 host 응답은 capacity cache에 임의의 계정을 만들지 않고 실패해야 합니다.
#[test]
fn rejects_account_capacity_without_an_email_identity() {
    let account_response = json!({
        "id": 2,
        "result": {"account": {"planType": "plus"}}
    });
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.149.1"),
        account_response,
        json!({"id": 3, "result": {"rateLimits": {}}}),
    ]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let failure = observe_account_capacity(&mut client).unwrap_err();

    assert!(failure.message().contains("no valid `email`"));
    assert!(
        sent.0
            .borrow()
            .iter()
            .all(|message| { message["method"] != "account/rateLimits/read" })
    );
}
