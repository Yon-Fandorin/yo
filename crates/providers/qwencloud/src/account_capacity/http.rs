use std::{str, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, RequestBuilder, Url, header, redirect, retry};
use serde_json::Value;
use tokio::{
    runtime::Builder,
    time::{Instant, timeout_at},
};
use yo_core::{AccountCapacitySnapshot, AccountId, ApiCredential, ProviderId};

use super::{
    decode::{decode_gateway_envelope, decode_snapshot, extract_sec_token, matches_media_type},
    error::{QwenCloudCapacityFailureKind, QwenCloudResult, expired_session_error, failure},
    model::{ExpectedMedia, QwenCloudProviderData},
};

pub(super) const LOGIN_COOKIE: &str = "login_qwencloud_ticket";
const DASHBOARD_URL: &str = "https://home.qwencloud.com/";
const GATEWAY_URL: &str = "https://cs-data.qwencloud.com/data/api.json";
const GATEWAY_PRODUCT: &str = "sfm_bailian";
const GATEWAY_ACTION: &str = "IntlBroadScopeAspnGateway";
const GATEWAY_REGION: &str = "ap-southeast-1";
const GATEWAY_API_PREFIX: &str = "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/";
const COMMODITY_CODE: &str = "sfm_tokenplansolo_public_intl";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn validate_account_session(cookie: ApiCredential) -> QwenCloudResult<ApiCredential> {
    if cookie_value(cookie.expose_secret(), LOGIN_COOKIE).is_none() {
        return Err(failure(
            QwenCloudCapacityFailureKind::Configuration,
            "QwenCloud browser Cookie has no non-empty login_qwencloud_ticket",
        ));
    }
    Ok(cookie)
}

pub(super) fn read_account_capacity(
    provider: &ProviderId,
    account: &AccountId,
    cookie: &ApiCredential,
) -> QwenCloudResult<(AccountCapacitySnapshot, QwenCloudProviderData)> {
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(retry::never())
        .build()
        .map_err(|_| {
            failure(
                QwenCloudCapacityFailureKind::Configuration,
                "cannot initialize the QwenCloud capacity HTTP client",
            )
        })?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            failure(
                QwenCloudCapacityFailureKind::Configuration,
                "cannot initialize the QwenCloud capacity runtime",
            )
        })?;

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
        .map_err(|_| {
            failure(
                QwenCloudCapacityFailureKind::Protocol,
                "QwenCloud console sec_token is invalid",
            )
        })?;
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
    decode_snapshot(&usage, &subscription, &quota_config, provider, account)
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
    let html = str::from_utf8(&bytes).map_err(|_| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            "QwenCloud dashboard response is not valid UTF-8",
        )
    })?;
    let token = extract_sec_token(html).ok_or_else(expired_session_error)?;
    ApiCredential::new(token.to_owned()).map_err(|_| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            "QwenCloud dashboard returned an invalid sec_token",
        )
    })
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

pub(super) fn build_gateway_request(
    client: &Client,
    cookie: &ApiCredential,
    sec_token: &ApiCredential,
    endpoint: &str,
) -> QwenCloudResult<RequestBuilder> {
    let api = format!("{GATEWAY_API_PREFIX}{endpoint}");
    let mut url = Url::parse(GATEWAY_URL).map_err(|_| {
        failure(
            QwenCloudCapacityFailureKind::Configuration,
            "the built-in QwenCloud gateway URL is invalid",
        )
    })?;
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
                "console": "ONE_CONSOLE", "consoleSite": "QWENCLOUD",
                "domain": "home.qwencloud.com", "productCode": "p_efm",
                "protocol": "V2", "xsp_lang": "en-US"
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

pub(super) async fn fetch_bounded(
    request: RequestBuilder,
    expected_media: ExpectedMedia,
) -> QwenCloudResult<Vec<u8>> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let response = timeout_at(deadline, request.send())
        .await
        .map_err(|_| {
            failure(
                QwenCloudCapacityFailureKind::Timeout,
                "QwenCloud capacity request deadline expired",
            )
        })?
        .map_err(|_| {
            failure(
                QwenCloudCapacityFailureKind::Transport,
                "QwenCloud capacity HTTP request failed",
            )
        })?;
    if response.status().is_redirection() {
        return Err(expired_session_error());
    }
    if !response.status().is_success() {
        return Err(failure(
            QwenCloudCapacityFailureKind::HttpStatus,
            format!(
                "QwenCloud capacity endpoint returned HTTP status {}",
                response.status().as_u16()
            ),
        ));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !matches_media_type(content_type, expected_media) {
        return Err(failure(
            QwenCloudCapacityFailureKind::MediaType,
            "QwenCloud capacity success returned an unexpected media type",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(failure(
            QwenCloudCapacityFailureKind::Limit,
            "QwenCloud capacity response exceeds 1 MiB",
        ));
    }
    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = deadline.min(body_progress + BODY_IDLE_TIMEOUT);
        let next = timeout_at(body_deadline, chunks.next())
            .await
            .map_err(|_| {
                failure(
                    QwenCloudCapacityFailureKind::Timeout,
                    "QwenCloud capacity response-body deadline expired",
                )
            })?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(failure(
                        QwenCloudCapacityFailureKind::Limit,
                        "QwenCloud capacity response exceeds 1 MiB",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(failure(
                    QwenCloudCapacityFailureKind::Transport,
                    "QwenCloud capacity response body could not be read",
                ));
            },
            None => return Ok(bytes),
        }
    }
}

pub(super) fn cookie_value<'a>(cookie: &'a str, name: &str) -> Option<&'a str> {
    cookie.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name && !value.is_empty()).then_some(value)
    })
}
