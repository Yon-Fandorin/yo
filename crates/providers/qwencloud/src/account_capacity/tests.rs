use super::*;

fn provider() -> ProviderId {
    ProviderId::new("qwencloud").unwrap()
}
fn account() -> AccountId {
    AccountId::new("default").unwrap()
}

fn gateway_payload(data: Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "code": "200", "data": { "DataV2": { "data": {
            "code": "SUCCESS", "success": true, "data": data
        }}}
    }))
    .unwrap()
}

// Gateway request는 QwenCloud identity와 personal Token Plan API를 exact query·form으로
// 보내고 console cookie는 header에만 두어 URL이나 payload에 복제하지 않습니다.
#[test]
fn builds_exact_qwencloud_gateway_request() {
    let client = Client::builder().build().unwrap();
    let cookie =
        ApiCredential::new("cna=x; login_qwencloud_ticket=google-session; auxiliary=1").unwrap();
    let sec_token = ApiCredential::new("sec-token").unwrap();
    let request = build_gateway_request(&client, &cookie, &sec_token, "usage")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().host_str(), Some("cs-data.qwencloud.com"));
    let query = request.url().query_pairs().collect::<Vec<_>>();
    assert!(query.iter().any(|(key, value)| key == "api" && value.ends_with("/tokenplan/personal/api/v2/usage")));
    assert_eq!(
        request.headers().get(header::ORIGIN).unwrap(),
        "https://home.qwencloud.com"
    );
    assert_eq!(
        request.headers().get(header::COOKIE).unwrap(),
        cookie.expose_secret()
    );
    let body = request.body().and_then(reqwest::Body::as_bytes).unwrap();
    let body = std::str::from_utf8(body).unwrap();
    assert!(body.contains("product=sfm_bailian"));
    assert!(body.contains("action=IntlBroadScopeAspnGateway"));
    assert!(body.contains("region=ap-southeast-1"));
    assert!(body.contains("sec_token=sec-token"));
    assert!(!body.contains("google-session"));
}

// QwenCloud console의 5시간·7일 비율과 reset, active tier를 공용 snapshot으로
// 보수 변환해 화면과 JSON이 같은 두 window를 소비하도록 합니다.
#[test]
fn decodes_personal_token_plan_windows_and_dynamic_tier() {
    let usage = serde_json::json!({ "per5HourPercentage": 0.251, "per5HourResetTime": 1_800_000_000_000_i64, "per1WeekPercentage": 0.55, "per1WeekResetTime": 1_800_500_000_000_i64 });
    let subscription = serde_json::json!({ "specCode": "moderato" });
    let quota_config = serde_json::json!({ "moderato": { "five_hour": 3_000, "weekly": 10_000 } });
    let (snapshot, provider_data) = decode_snapshot(
        &usage,
        &subscription,
        &quota_config,
        &provider(),
        &account(),
    )
    .unwrap();
    let bucket = &snapshot.buckets()[0];
    assert_eq!(bucket.plan(), Some("moderato"));
    assert_eq!(
        bucket.primary().unwrap().window_duration_minutes(),
        Some(300)
    );
    assert_eq!(bucket.primary().unwrap().used_percent(), 26);
    assert_eq!(bucket.primary().unwrap().used_percent_basis_points(), 2_510);
    assert_eq!(
        bucket.secondary().unwrap().window_duration_minutes(),
        Some(10_080)
    );
    assert_eq!(bucket.secondary().unwrap().used_percent(), 55);
    assert_eq!(
        bucket.secondary().unwrap().used_percent_basis_points(),
        5_500
    );
    assert_eq!(
        bucket.secondary().unwrap().resets_at_unix_seconds(),
        Some(1_800_500_000)
    );
    assert_eq!(
        serde_json::to_value(provider_data).unwrap(),
        serde_json::json!({
            "specCode": "moderato", "usage": { "per5HourPercentage": 0.251, "per5HourResetTime": 1_800_000_000_000_i64, "per1WeekPercentage": 0.55, "per1WeekResetTime": 1_800_500_000_000_i64 },
            "quota": { "five_hour": 3_000, "weekly": 10_000 }
        })
    );
}

// Provider가 일시 제거한 5시간 field는 빈 값으로 합성하지 않고, 실제로 남아 있는
// 7일 window만 primary로 승격하여 usable 관측을 보존합니다.
#[test]
fn accepts_weekly_window_when_five_hour_window_is_omitted() {
    let usage =
        serde_json::json!({ "per1WeekPercentage": "0.4", "per1WeekResetTime": 1_800_500_000_i64 });
    let subscription = serde_json::json!({ "specCode": "lite" });
    let quota_config = serde_json::json!({ "lite": { "weekly": 2_500 } });
    let (snapshot, provider_data) = decode_snapshot(
        &usage,
        &subscription,
        &quota_config,
        &provider(),
        &account(),
    )
    .unwrap();
    let bucket = &snapshot.buckets()[0];
    assert_eq!(
        bucket.primary().unwrap().window_duration_minutes(),
        Some(10_080)
    );
    assert_eq!(bucket.primary().unwrap().used_percent(), 40);
    assert!(bucket.secondary().is_none());
    assert_eq!(
        serde_json::to_value(provider_data).unwrap(),
        serde_json::json!({
            "specCode": "lite", "usage": { "per1WeekPercentage": "0.4", "per1WeekResetTime": 1_800_500_000_i64 }, "quota": { "weekly": 2_500 }
        })
    );
}

// Login redirect와 malformed envelope는 정상 quota로 해석하지 않으며, payload parser는
// console gateway가 성공을 명시한 exact nested data만 내보냅니다.
#[test]
fn unwraps_only_successful_gateway_envelopes() {
    let payload = serde_json::json!({ "specCode": "moderato" });
    assert_eq!(
        decode_gateway_envelope(&gateway_payload(payload.clone())).unwrap(),
        payload
    );
    let expired = decode_gateway_envelope(br#"{"code":"ConsoleNeedLogin"}"#).unwrap_err();
    assert!(expired.is_expired_session());
    for bytes in [
        br#"{"code":"200","data":{"DataV2":{"data":{"code":"FAILED","success":false}}}}"#
            .as_slice(),
        br#"not-json"#.as_slice(),
    ] {
        assert!(decode_gateway_envelope(bytes).is_err());
    }
}

// 전체 Cookie header에서 exact QwenCloud login ticket과 optional sec_token만 이름으로
// 고르고, 비슷한 이름이나 빈 값은 인증 material로 받아들이지 않습니다.
#[test]
fn resolves_exact_cookie_fields() {
    let cookie = "cna=x; login_qwencloud_ticket=google-session; sec_token=token-1";
    assert_eq!(cookie_value(cookie, LOGIN_COOKIE), Some("google-session"));
    assert_eq!(cookie_value(cookie, "sec_token"), Some("token-1"));
    assert_eq!(
        cookie_value("login_qwencloud_ticket_extra=x", LOGIN_COOKIE),
        None
    );
    assert_eq!(cookie_value("login_qwencloud_ticket=", LOGIN_COOKIE), None);
    assert!(validate_account_session(ApiCredential::new(cookie).unwrap()).is_ok());
    assert!(validate_account_session(ApiCredential::new("cna=x").unwrap()).is_err());
}

// Dashboard HTML에서 bounded quoted SEC_TOKEN만 추출하여 다른 script text나 control
// character를 console 요청 credential로 승격하지 않습니다.
#[test]
fn extracts_only_quoted_dashboard_sec_token() {
    assert_eq!(
        extract_sec_token(r#"<script>window.X={SEC_TOKEN: "resolved-token"}</script>"#),
        Some("resolved-token")
    );
    assert_eq!(extract_sec_token("SEC_TOKEN: unquoted"), None);
    assert_eq!(extract_sec_token("<html>none</html>"), None);
}

// 비율·active tier·quota config가 서로 어긋나면 부분적인 healthy report를 만들지 않아
// 오래된 console 응답 shape를 사용자가 최신 잔여량으로 오인하지 않게 합니다.
#[test]
fn rejects_invalid_percentage_or_missing_active_tier() {
    let subscription = serde_json::json!({ "specCode": "moderato" });
    let quota_config = serde_json::json!({ "moderato": { "five_hour": 3_000, "weekly": 10_000 } });
    assert!(decode_snapshot(&serde_json::json!({ "per1WeekPercentage": 1.1, "per1WeekResetTime": 1_800_500_000_i64 }), &subscription, &quota_config, &provider(), &account()).is_err());
    assert!(decode_snapshot(&serde_json::json!({ "per1WeekPercentage": 0.5, "per1WeekResetTime": 1_800_500_000_i64 }), &subscription, &serde_json::json!({}), &provider(), &account()).is_err());
}
