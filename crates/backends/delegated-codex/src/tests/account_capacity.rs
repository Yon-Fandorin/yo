use std::time::Duration;

use serde_json::json;

use super::{
    super::{client::AppServerClient, observe_account_capacity},
    support::{FakePeer, initialize_response},
};

// 계정 조회는 Agent thread를 만들지 않고 initialize 뒤 정확히 한 번의 읽기 RPC만 보내며,
// 다중 limit bucket을 안정된 key 순서와 공용 용량 값으로 변환합니다.
#[test]
fn reads_account_capacity_without_creating_a_session() {
    let response = json!({
        "id": 2,
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
    let (peer, sent) = FakePeer::new([initialize_response(1, "0.149.1"), response]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let snapshot = observe_account_capacity(&mut client).unwrap();

    assert_eq!(snapshot.provider().as_str(), "codex");
    assert_eq!(snapshot.account().as_str(), "default");
    assert_eq!(snapshot.buckets().len(), 2);
    assert_eq!(snapshot.buckets()[0].id(), Some("codex"));
    assert_eq!(snapshot.buckets()[0].plan(), Some("plus"));
    assert_eq!(
        snapshot.buckets()[0].primary().unwrap().remaining_percent(),
        63
    );
    assert_eq!(snapshot.buckets()[1].id(), Some("z-extra"));

    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2]["method"], "account/rateLimits/read");
    assert!(sent[2]["params"].is_null());
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
    let response = json!({
        "id": 2,
        "result": {
            "rateLimits": {
                "primary": { "usedPercent": 101 }
            }
        }
    });
    let (peer, _) = FakePeer::new([initialize_response(1, "0.149.1"), response]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let failure = observe_account_capacity(&mut client).unwrap_err();

    assert!(failure.message().contains("between 0 and 100"));
}
