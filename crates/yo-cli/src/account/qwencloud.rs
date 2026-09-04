use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, RequestBuilder, Url, header, redirect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::{Instant, timeout_at};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountId,
    ApiCredential, ProviderId,
};

use crate::AppError;

const LOGIN_COOKIE: &str = "login_qwencloud_ticket";
const DASHBOARD_URL: &str = "https://home.qwencloud.com/";
const GATEWAY_URL: &str = "https://cs-data.qwencloud.com/data/api.json";
const GATEWAY_PRODUCT: &str = "sfm_bailian";
const GATEWAY_ACTION: &str = "IntlBroadScopeAspnGateway";
const GATEWAY_REGION: &str = "ap-southeast-1";
const GATEWAY_API_PREFIX: &str = "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/";
const COMMODITY_CODE: &str = "sfm_tokenplansolo_public_intl";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_PLAN_BYTES: usize = 128;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const FIVE_HOURS_MINUTES: u64 = 5 * 60;
const ONE_WEEK_MINUTES: u64 = 7 * 24 * 60;

#[derive(Clone, Copy)]
enum ExpectedMedia {
    Html,
    Json,
}

pub(super) struct QwenCloudCapacityError {
    kind: QwenCloudCapacityErrorKind,
    source: AppError,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QwenCloudProviderData {
    spec_code: String,
    usage: QwenCloudUsageData,
    quota: QwenCloudQuotaData,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QwenCloudUsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    per5_hour_percentage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per5_hour_reset_time: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per1_week_percentage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per1_week_reset_time: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct QwenCloudQuotaData {
    #[serde(skip_serializing_if = "Option::is_none")]
    five_hour: Option<Value>,
    weekly: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QwenCloudCapacityErrorKind {
    ExpiredSession,
    Other,
}

impl QwenCloudCapacityError {
    pub(super) fn is_expired_session(&self) -> bool {
        self.kind == QwenCloudCapacityErrorKind::ExpiredSession
    }

    pub(super) fn into_app_error(self) -> AppError {
        self.source
    }
}

impl From<AppError> for QwenCloudCapacityError {
    fn from(source: AppError) -> Self {
        Self {
            kind: QwenCloudCapacityErrorKind::Other,
            source,
        }
    }
}

impl std::fmt::Debug for QwenCloudCapacityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QwenCloudCapacityError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

type QwenCloudResult<T> = Result<T, QwenCloudCapacityError>;

pub(super) fn validate_account_session(cookie: ApiCredential) -> Result<ApiCredential, AppError> {
    if cookie_value(cookie.expose_secret(), LOGIN_COOKIE).is_none() {
        return Err(AppError::message(
            "QwenCloud browser Cookie has no non-empty login_qwencloud_ticket",
        ));
    }
    Ok(cookie)
}

/// Reads QwenCloud Personal Token Plan capacity through its authenticated console session.
///
/// The inference `sk-sp-*` key cannot authorize the console gateway. The caller supplies the
/// exact account-session secret captured from the local credential snapshot.
pub(super) fn read_account_capacity(
    provider: &ProviderId,
    account: &AccountId,
    cookie: &ApiCredential,
) -> QwenCloudResult<(AccountCapacitySnapshot, QwenCloudProviderData)> {
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| AppError::message("cannot initialize the QwenCloud capacity HTTP client"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| AppError::message("cannot initialize the QwenCloud capacity runtime"))?;

    runtime.block_on(read_remote_snapshot(&client, provider, account, cookie))
}

async fn read_remote_snapshot(
    client: &Client,
    provider: &ProviderId,
    account: &AccountId,
    cookie: &ApiCredential,
) -> QwenCloudResult<(AccountCapacitySnapshot, QwenCloudProviderData)> {
    let cookie_sec_token = cookie_value(cookie.expose_secret(), "sec_token")
        .map(str::to_owned)
        .map(ApiCredential::new)
        .transpose()
        .map_err(|_| AppError::message("QwenCloud console sec_token is invalid"))?;
    let resolved_sec_token;
    let sec_token = if let Some(token) = cookie_sec_token.as_ref() {
        token
    } else {
        resolved_sec_token = resolve_sec_token(client, cookie).await?;
        &resolved_sec_token
    };

    let (usage, subscription, quota_config) = tokio::try_join!(
        call_gateway(client, cookie, sec_token, "usage"),
        call_gateway(client, cookie, sec_token, "subscription"),
        call_gateway(client, cookie, sec_token, "quota-config"),
    )?;
    decode_snapshot(&usage, &subscription, &quota_config, provider, account).map_err(Into::into)
}

async fn resolve_sec_token(
    client: &Client,
    cookie: &ApiCredential,
) -> QwenCloudResult<ApiCredential> {
    let request = client
        .get(DASHBOARD_URL)
        .header(header::COOKIE, cookie.expose_secret())
        .header(header::ACCEPT, "text/html")
        .header(
            header::USER_AGENT,
            "Mozilla/5.0 AppleWebKit/537.36 Chrome/126.0 Safari/537.36",
        );
    let bytes = fetch_bounded(request, ExpectedMedia::Html).await?;
    let html = std::str::from_utf8(&bytes)
        .map_err(|_| AppError::message("QwenCloud dashboard response is not valid UTF-8"))?;
    let token = extract_sec_token(html).ok_or_else(expired_session_error)?;
    Ok(ApiCredential::new(token.to_owned())
        .map_err(|_| AppError::message("QwenCloud dashboard returned an invalid sec_token"))?)
}

async fn call_gateway(
    client: &Client,
    cookie: &ApiCredential,
    sec_token: &ApiCredential,
    endpoint: &str,
) -> QwenCloudResult<Value> {
    let request = build_gateway_request(client, cookie, sec_token, endpoint)?;
    let bytes = fetch_bounded(request, ExpectedMedia::Json).await?;
    decode_gateway_envelope(&bytes)
}

fn build_gateway_request(
    client: &Client,
    cookie: &ApiCredential,
    sec_token: &ApiCredential,
    endpoint: &str,
) -> QwenCloudResult<RequestBuilder> {
    let api = format!("{GATEWAY_API_PREFIX}{endpoint}");
    let mut url = Url::parse(GATEWAY_URL)
        .map_err(|_| AppError::message("the built-in QwenCloud gateway URL is invalid"))?;
    url.query_pairs_mut()
        .append_pair("product", GATEWAY_PRODUCT)
        .append_pair("action", GATEWAY_ACTION)
        .append_pair("api", &api);
    let params = serde_json::json!({
        "Api": api,
        "V": "1.0",
        "Data": {
            "commodityCode": COMMODITY_CODE,
            "cornerstoneParam": {
                "console": "ONE_CONSOLE",
                "consoleSite": "QWENCLOUD",
                "domain": "home.qwencloud.com",
                "productCode": "p_efm",
                "protocol": "V2",
                "xsp_lang": "en-US"
            }
        }
    })
    .to_string();
    let form = [
        ("product", GATEWAY_PRODUCT),
        ("action", GATEWAY_ACTION),
        ("sec_token", sec_token.expose_secret()),
        ("region", GATEWAY_REGION),
        ("params", params.as_str()),
    ];
    Ok(client
        .post(url)
        .header(header::COOKIE, cookie.expose_secret())
        .header(header::ACCEPT, "application/json")
        .header(header::ORIGIN, "https://home.qwencloud.com")
        .header(header::REFERER, "https://home.qwencloud.com/")
        .form(&form))
}

async fn fetch_bounded(
    request: RequestBuilder,
    expected_media: ExpectedMedia,
) -> QwenCloudResult<Vec<u8>> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let response = timeout_at(deadline, request.send())
        .await
        .map_err(|_| AppError::message("QwenCloud capacity request deadline expired"))?
        .map_err(|_| AppError::message("QwenCloud capacity HTTP request failed"))?;
    if response.status().is_redirection() {
        return Err(expired_session_error());
    }
    if !response.status().is_success() {
        return Err(AppError::message(format!(
            "QwenCloud capacity endpoint returned HTTP status {}",
            response.status().as_u16()
        ))
        .into());
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !matches_media_type(content_type, expected_media) {
        return Err(AppError::message(
            "QwenCloud capacity success returned an unexpected media type",
        )
        .into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(AppError::message("QwenCloud capacity response exceeds 1 MiB").into());
    }

    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = deadline.min(body_progress + BODY_IDLE_TIMEOUT);
        let next = timeout_at(body_deadline, chunks.next())
            .await
            .map_err(|_| AppError::message("QwenCloud capacity response-body deadline expired"))?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(
                        AppError::message("QwenCloud capacity response exceeds 1 MiB").into(),
                    );
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(AppError::message(
                    "QwenCloud capacity response body could not be read",
                )
                .into());
            },
            None => return Ok(bytes),
        }
    }
}

fn decode_gateway_envelope(bytes: &[u8]) -> QwenCloudResult<Value> {
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|_| AppError::message("QwenCloud console response is not valid JSON"))?;
    let root_code = root.get("code");
    if root_code.and_then(Value::as_str) == Some("ConsoleNeedLogin") {
        return Err(expired_session_error());
    }
    let root_success = root_code.and_then(Value::as_str) == Some("200")
        || root_code.and_then(Value::as_u64) == Some(200);
    let inner = root
        .pointer("/data/DataV2/data")
        .ok_or_else(|| AppError::message("QwenCloud console response has no gateway result"))?;
    if !root_success
        || inner.get("code").and_then(Value::as_str) != Some("SUCCESS")
        || inner.get("success").and_then(Value::as_bool) != Some(true)
    {
        return Err(AppError::message(
            "QwenCloud console rejected the capacity request; the browser session may have expired",
        )
        .into());
    }
    Ok(inner
        .get("data")
        .cloned()
        .ok_or_else(|| AppError::message("QwenCloud console response has no capacity payload"))?)
}

fn decode_snapshot(
    usage: &Value,
    subscription: &Value,
    quota_config: &Value,
    provider: &ProviderId,
    account: &AccountId,
) -> Result<(AccountCapacitySnapshot, QwenCloudProviderData), AppError> {
    let spec_code = bounded_text(subscription.get("specCode"), "subscription specCode")?;
    let tier = quota_config
        .get(spec_code)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::message("QwenCloud quota configuration has no active subscription tier")
        })?;
    let weekly_quota = validate_positive_number(tier.get("weekly"), "weekly quota")?;
    if tier.contains_key("five_hour") {
        validate_positive_number(tier.get("five_hour"), "five-hour quota")?;
    }

    let five_hour = decode_window(usage, "5Hour", FIVE_HOURS_MINUTES)?;
    let weekly = decode_window(usage, "1Week", ONE_WEEK_MINUTES)?;
    let (primary, secondary) = match (five_hour, weekly) {
        (Some(five_hour), Some(weekly)) => (Some(five_hour), Some(weekly)),
        (Some(five_hour), None) => (Some(five_hour), None),
        (None, Some(weekly)) => (Some(weekly), None),
        (None, None) => {
            return Err(AppError::message(
                "QwenCloud usage response contains no usable quota window",
            ));
        },
    };
    let limited = primary
        .iter()
        .chain(secondary.iter())
        .any(|window| window.used_percent_basis_points() == 10_000);
    let bucket = AccountCapacityBucket::new(
        Some("qwencloud".to_owned()),
        Some("QwenCloud Token Plan".to_owned()),
        Some(spec_code.to_owned()),
        primary,
        secondary,
        None,
        limited.then(|| "usage_limit_reached".to_owned()),
    );
    let provider_data = QwenCloudProviderData {
        spec_code: spec_code.to_owned(),
        usage: QwenCloudUsageData {
            per5_hour_percentage: five_hour
                .as_ref()
                .and_then(|_| usage.get("per5HourPercentage").cloned()),
            per5_hour_reset_time: five_hour
                .as_ref()
                .and_then(|_| usage.get("per5HourResetTime").cloned()),
            per1_week_percentage: weekly
                .as_ref()
                .and_then(|_| usage.get("per1WeekPercentage").cloned()),
            per1_week_reset_time: weekly
                .as_ref()
                .and_then(|_| usage.get("per1WeekResetTime").cloned()),
        },
        quota: QwenCloudQuotaData {
            five_hour: tier.get("five_hour").cloned(),
            weekly: weekly_quota.clone(),
        },
    };
    Ok((
        AccountCapacitySnapshot::new(provider.clone(), account.clone(), vec![bucket]),
        provider_data,
    ))
}

fn decode_window(
    usage: &Value,
    field_prefix: &str,
    duration_minutes: u64,
) -> Result<Option<AccountCapacityWindow>, AppError> {
    let percentage_field = format!("per{field_prefix}Percentage");
    let reset_field = format!("per{field_prefix}ResetTime");
    let Some(raw_percentage) = usage.get(&percentage_field) else {
        return Ok(None);
    };
    let percentage = number(raw_percentage).ok_or_else(|| {
        AppError::message(format!("QwenCloud usage {percentage_field} is not numeric"))
    })?;
    if !percentage.is_finite() || !(0.0..=1.0).contains(&percentage) {
        return Err(AppError::message(format!(
            "QwenCloud usage {percentage_field} is outside 0..1"
        )));
    }
    let used_percent_basis_points = (percentage * 10_000.0).ceil().min(10_000.0) as u16;
    let reset = usage
        .get(&reset_field)
        .map(|value| normalize_reset(value, &reset_field))
        .transpose()?;
    AccountCapacityWindow::from_used_percent_basis_points(
        used_percent_basis_points,
        Some(duration_minutes),
        reset,
    )
    .map(Some)
    .map_err(|error| AppError::single("normalizing QwenCloud capacity", error))
}

fn normalize_reset(value: &Value, field: &str) -> Result<i64, AppError> {
    let value = value
        .as_i64()
        .ok_or_else(|| AppError::message(format!("QwenCloud usage {field} is not an integer")))?;
    if value <= 0 {
        return Err(AppError::message(format!(
            "QwenCloud usage {field} must be positive"
        )));
    }
    Ok(if value >= 100_000_000_000 {
        value / 1_000
    } else {
        value
    })
}

fn bounded_text<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str, AppError> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::message(format!("QwenCloud {field} is not a string")))?;
    if value.is_empty() || value.len() > MAX_PLAN_BYTES || value.chars().any(char::is_control) {
        return Err(AppError::message(format!(
            "QwenCloud {field} is outside the bounded text profile"
        )));
    }
    Ok(value)
}

fn validate_positive_number<'a>(
    value: Option<&'a Value>,
    field: &str,
) -> Result<&'a Value, AppError> {
    let source =
        value.ok_or_else(|| AppError::message(format!("QwenCloud {field} is not numeric")))?;
    let value = number(source)
        .ok_or_else(|| AppError::message(format!("QwenCloud {field} is not numeric")))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(AppError::message(format!(
            "QwenCloud {field} must be finite and positive"
        )));
    }
    Ok(source)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn cookie_value<'a>(cookie: &'a str, name: &str) -> Option<&'a str> {
    cookie.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name && !value.is_empty()).then_some(value)
    })
}

fn extract_sec_token(html: &str) -> Option<&str> {
    for marker in ["SEC_TOKEN", "SEC-TOKEN"] {
        let mut rest = html;
        while let Some(index) = rest.find(marker) {
            rest = &rest[index + marker.len()..];
            let probe = &rest[..rest.len().min(512)];
            let Some(separator) = probe.find([':', '=']) else {
                continue;
            };
            let candidate = rest[separator + 1..].trim_start();
            let Some(quote) = candidate.chars().next() else {
                continue;
            };
            if quote != '\'' && quote != '"' {
                continue;
            }
            let quoted = &candidate[quote.len_utf8()..];
            let Some(end) = quoted.find(quote) else {
                continue;
            };
            let token = &quoted[..end];
            if !token.is_empty() && !token.chars().any(char::is_control) {
                return Some(token);
            }
        }
    }
    None
}

fn matches_media_type(content_type: &str, expected: ExpectedMedia) -> bool {
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    match expected {
        ExpectedMedia::Html => media_type.eq_ignore_ascii_case("text/html"),
        ExpectedMedia::Json => {
            media_type.eq_ignore_ascii_case("application/json")
                || media_type
                    .to_ascii_lowercase()
                    .strip_prefix("application/")
                    .is_some_and(|subtype| subtype.ends_with("+json"))
        },
    }
}

pub(super) fn expired_session_error() -> QwenCloudCapacityError {
    QwenCloudCapacityError {
        kind: QwenCloudCapacityErrorKind::ExpiredSession,
        source: AppError::message(
            "QwenCloud console session expired; enter a new browser Cookie. The stored model connection and API key are unchanged",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> ProviderId {
        ProviderId::new("qwencloud").unwrap()
    }

    fn account() -> AccountId {
        AccountId::new("default").unwrap()
    }

    fn gateway_payload(data: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "code": "200",
            "data": {
                "DataV2": {
                    "data": {
                        "code": "SUCCESS",
                        "success": true,
                        "data": data
                    }
                }
            }
        }))
        .unwrap()
    }

    // Gateway request는 QwenCloud identity와 personal Token Plan API를 exact query·form으로
    // 보내고 console cookie는 header에만 두어 URL이나 payload에 복제하지 않습니다.
    #[test]
    fn builds_exact_qwencloud_gateway_request() {
        let client = Client::builder().build().unwrap();
        let cookie =
            ApiCredential::new("cna=x; login_qwencloud_ticket=google-session; auxiliary=1")
                .unwrap();
        let sec_token = ApiCredential::new("sec-token").unwrap();
        let request = build_gateway_request(&client, &cookie, &sec_token, "usage")
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(request.url().host_str(), Some("cs-data.qwencloud.com"));
        let query = request.url().query_pairs().collect::<Vec<_>>();
        assert!(query.iter().any(|(key, value)| {
            key == "api" && value.ends_with("/tokenplan/personal/api/v2/usage")
        }));
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
        let usage = serde_json::json!({
            "per5HourPercentage": 0.251,
            "per5HourResetTime": 1_800_000_000_000_i64,
            "per1WeekPercentage": 0.55,
            "per1WeekResetTime": 1_800_500_000_000_i64
        });
        let subscription = serde_json::json!({ "specCode": "moderato" });
        let quota_config = serde_json::json!({
            "moderato": { "five_hour": 3_000, "weekly": 10_000 }
        });

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
                "specCode": "moderato",
                "usage": {
                    "per5HourPercentage": 0.251,
                    "per5HourResetTime": 1_800_000_000_000_i64,
                    "per1WeekPercentage": 0.55,
                    "per1WeekResetTime": 1_800_500_000_000_i64
                },
                "quota": { "five_hour": 3_000, "weekly": 10_000 }
            })
        );
    }

    // Provider가 일시 제거한 5시간 field는 빈 값으로 합성하지 않고, 실제로 남아 있는
    // 7일 window만 primary로 승격하여 usable 관측을 보존합니다.
    #[test]
    fn accepts_weekly_window_when_five_hour_window_is_omitted() {
        let usage = serde_json::json!({
            "per1WeekPercentage": "0.4",
            "per1WeekResetTime": 1_800_500_000_i64
        });
        let subscription = serde_json::json!({ "specCode": "lite" });
        let quota_config = serde_json::json!({
            "lite": { "weekly": 2_500 }
        });

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
                "specCode": "lite",
                "usage": {
                    "per1WeekPercentage": "0.4",
                    "per1WeekResetTime": 1_800_500_000_i64
                },
                "quota": { "weekly": 2_500 }
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
        let quota_config = serde_json::json!({
            "moderato": { "five_hour": 3_000, "weekly": 10_000 }
        });
        assert!(
            decode_snapshot(
                &serde_json::json!({
                    "per1WeekPercentage": 1.1,
                    "per1WeekResetTime": 1_800_500_000_i64
                }),
                &subscription,
                &quota_config,
                &provider(),
                &account(),
            )
            .is_err()
        );
        assert!(
            decode_snapshot(
                &serde_json::json!({
                    "per1WeekPercentage": 0.5,
                    "per1WeekResetTime": 1_800_500_000_i64
                }),
                &subscription,
                &serde_json::json!({}),
                &provider(),
                &account(),
            )
            .is_err()
        );
    }
}
