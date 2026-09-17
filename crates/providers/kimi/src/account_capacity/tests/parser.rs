use serde_json::json;

use super::{
    super::{
        decode::{parse_kimi_account_capacity_snapshot_with_plan, parse_kimi_account_plan},
        error::KimiAccountCapacityFailureKind,
        model::{MAX_RESPONSE_BYTES, MINUTES_PER_WEEK},
        parse_kimi_account_capacity_snapshot,
    },
    code_seed, platform_seed, usage_payload,
};

// Kimi Code의 weekly summary와 rolling limit을 공용 두 window로 투영하고,
// count 비율은 남은 양을 과장하지 않도록 올림한 used percentage로 정규화합니다.
#[test]
fn decodes_weekly_rolling_and_booster_capacity() {
    let snapshot = parse_kimi_account_capacity_snapshot(&code_seed(), &usage_payload()).unwrap();
    assert_eq!(snapshot.provider().as_str(), "kimi");
    assert_eq!(snapshot.account().as_str(), "default");
    let bucket = &snapshot.buckets()[0];
    let weekly = bucket.primary().unwrap();
    let rolling = bucket.secondary().unwrap();
    assert_eq!(weekly.used_percent(), 92);
    assert_eq!(weekly.window_duration_minutes(), Some(MINUTES_PER_WEEK));
    assert_eq!(rolling.used_percent(), 7);
    assert_eq!(rolling.window_duration_minutes(), Some(300));
    assert_eq!(bucket.credits().unwrap().balance(), Some("USD 5.00"));
    assert_eq!(bucket.limit_reason(), None);
}

// Kimi Code의 `/me`가 보고한 사용자 등급명은 공개 플랜명과 같은 값을 가지므로
// 별도 추정 없이 모든 계정 한도 bucket의 공용 plan 필드로 전달합니다.
#[test]
fn decodes_account_level_name_as_the_capacity_plan() {
    let plan = parse_kimi_account_plan(
        &serde_json::to_vec(&json!({
            "user_level": 20,
            "user_level_name": "Moderato"
        }))
        .unwrap(),
    )
    .unwrap();
    let snapshot =
        parse_kimi_account_capacity_snapshot_with_plan(&code_seed(), &usage_payload(), Some(plan))
            .unwrap();

    assert_eq!(snapshot.buckets()[0].plan(), Some("Moderato"));
    assert!(
        snapshot
            .buckets()
            .iter()
            .all(|bucket| bucket.plan() == Some("Moderato"))
    );
}

// `/me` 성공 응답에 등급명이 없거나 표시 경계를 벗어난 값이 있으면 플랜을
// 만들어내지 않고 protocol failure로 닫아 Unknown과 유효한 등급을 혼동하지 않습니다.
#[test]
fn rejects_missing_or_unsafe_account_level_names() {
    for payload in [
        json!({}),
        json!({"user_level_name": null}),
        json!({"user_level_name": ""}),
        json!({"user_level_name": "unsafe\nname"}),
    ] {
        let error = parse_kimi_account_plan(&serde_json::to_vec(&payload).unwrap()).unwrap_err();
        assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Protocol);
    }
}

// 성공 응답이어도 usable capacity가 없거나 배열·window가 계약 밖이면 조용히
// Unknown으로 만들지 않고 protocol failure로 닫습니다.
#[test]
fn rejects_empty_and_malformed_success_snapshots() {
    for payload in [
        json!({}),
        json!({"limits": {}}),
        json!({"limits": [{"window": {"duration": 5, "timeUnit": "HOUR"}, "detail": {"used": 1, "limit": 2}}]}),
        json!({"usage": {"used": 1, "limit": 0}}),
    ] {
        let error = parse_kimi_account_capacity_snapshot(
            &code_seed(),
            &serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Protocol);
    }
}

// 공식 Kimi Code parser처럼 limit이 있는 row에서 생략된 used는 아직 사용하지 않은
// 0으로 해석하지만, 분모인 limit 누락은 유효한 잔여량을 만들 수 없으므로 거절합니다.
#[test]
fn accepts_omitted_used_but_not_omitted_limit() {
    let snapshot = parse_kimi_account_capacity_snapshot(
        &code_seed(),
        &serde_json::to_vec(&json!({"usage": {"limit": "100"}})).unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot.buckets()[0].primary().unwrap().used_percent(), 0);

    let error = parse_kimi_account_capacity_snapshot(
        &code_seed(),
        &serde_json::to_vec(&json!({"usage": {"used": "1"}})).unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Protocol);
}

// `/usages`는 Kimi Code Membership의 계약이므로 같은 ProviderId라도 Platform
// API key 계정에는 요청 전 parser 경계에서 적용되지 않습니다.
#[test]
fn rejects_kimi_platform_accounts() {
    let error =
        parse_kimi_account_capacity_snapshot(&platform_seed(), &usage_payload()).unwrap_err();
    assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Configuration);
}

// direct parser와 transport가 같은 1 MiB 상한을 가져 큰 원격 JSON을 부분 결과로
// 해석하거나 호출 경로에 따라 메모리 한도를 달리하지 않습니다.
#[test]
fn direct_parser_keeps_the_response_byte_limit() {
    let error =
        parse_kimi_account_capacity_snapshot(&code_seed(), &vec![b' '; MAX_RESPONSE_BYTES + 1])
            .unwrap_err();
    assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Limit);
}
