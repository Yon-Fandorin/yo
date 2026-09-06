use std::io::Write;

use super::*;

// 큰 unified log 전체를 읽지 않고 bounded tail의 최신 완전한 주간 관찰만 선택합니다.
#[test]
fn reads_the_newest_complete_weekly_snapshot_from_a_bounded_tail() {
    let path = std::env::temp_dir().join(format!("yo-grok-billing-{}.jsonl", std::process::id()));
    let mut file = File::create(&path).unwrap();
    writeln!(file, r#"{{"msg":"unrelated","ctx":{{}}}}"#).unwrap();
    writeln!(
        file,
        r#"{{"msg":"billing: fetched credits config","ctx":{{"config":{{"creditUsagePercent":12.1,"currentPeriod":{{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2999-09-01T14:45:00Z"}}}}}}}}"#
    )
    .unwrap();

    let window = read_latest_usage(&path).unwrap().unwrap();
    std::fs::remove_file(path).unwrap();

    assert_eq!(window.used_percent(), 13);
    assert_eq!(window.used_percent_basis_points(), 1_210);
    assert_eq!(window.remaining_percent_basis_points(), 8_790);
    assert_eq!(window.window_duration_minutes(), Some(WEEKLY_MINUTES));
}

// proto3가 0을 생략하는 경우에도 유효한 current period가 있을 때만 0%로 해석합니다.
#[test]
fn treats_an_omitted_proto_zero_percentage_as_zero_only_with_a_valid_period() {
    let event = serde_json::json!({
        "msg": BILLING_MESSAGE,
        "ctx": { "config": { "currentPeriod": {
            "type": "USAGE_PERIOD_TYPE_WEEKLY",
            "end": "2999-09-01T14:45:00Z"
        }}}
    });

    assert_eq!(decode_usage(&event).unwrap().unwrap().used_percent(), 0);
    assert!(
        decode_usage(&serde_json::json!({ "msg": BILLING_MESSAGE }))
            .unwrap()
            .is_none()
    );
}
