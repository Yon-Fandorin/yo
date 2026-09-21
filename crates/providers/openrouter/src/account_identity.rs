use std::{
    error::Error,
    fmt,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{Client, ClientBuilder, StatusCode, Url, header, redirect, retry};
use serde::Deserialize;
use tokio::{
    runtime::Builder,
    time::{Instant as TokioInstant, timeout_at},
};
use yo_core::ApiCredential;

const CURRENT_KEY_URL: &str = "https://openrouter.ai/api/v1/key";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const MAX_ACCOUNT_COMPONENT_BYTES: usize = 128;

/// 현재 키 조회 중 실패한 경계입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterAccountIdentityFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    MediaType,
    Limit,
    Protocol,
    Timeout,
}

/// 현재 키 조회의 비밀정보를 포함하지 않는 오류입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRouterAccountIdentityError {
    kind: OpenRouterAccountIdentityFailureKind,
    message: &'static str,
}

impl OpenRouterAccountIdentityError {
    /// 오류가 발생한 경계입니다.
    #[must_use]
    pub const fn kind(&self) -> OpenRouterAccountIdentityFailureKind {
        self.kind
    }
}

impl fmt::Display for OpenRouterAccountIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl Error for OpenRouterAccountIdentityError {}

fn failure(
    kind: OpenRouterAccountIdentityFailureKind,
    message: &'static str,
) -> OpenRouterAccountIdentityError {
    OpenRouterAccountIdentityError { kind, message }
}

/// 저장된 OpenRouter 키를 인증하고 사용자·조직·작업공간의 정확한 결합 식별자를 반환합니다.
pub fn observe_openrouter_account_id(
    credential: &ApiCredential,
) -> Result<String, OpenRouterAccountIdentityError> {
    let url = Url::parse(CURRENT_KEY_URL).map_err(|_| {
        failure(
            OpenRouterAccountIdentityFailureKind::Configuration,
            "OpenRouter current-key URL is invalid",
        )
    })?;
    let client = current_key_client().build().map_err(|_| {
        failure(
            OpenRouterAccountIdentityFailureKind::Configuration,
            "cannot initialize the OpenRouter current-key HTTP client",
        )
    })?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            failure(
                OpenRouterAccountIdentityFailureKind::Transport,
                "cannot initialize the OpenRouter current-key request runtime",
            )
        })?;
    let bytes = runtime.block_on(fetch_current_key(&client, url, credential))?;
    parse_current_key(&bytes)
}

fn current_key_client() -> ClientBuilder {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(retry::never())
}

async fn fetch_current_key(
    client: &Client,
    url: Url,
    credential: &ApiCredential,
) -> Result<Vec<u8>, OpenRouterAccountIdentityError> {
    let absolute_deadline = Instant::now() + ABSOLUTE_TIMEOUT;
    let header_deadline = absolute_deadline.min(Instant::now() + RESPONSE_HEADER_TIMEOUT);
    let response = timeout_at(
        TokioInstant::from_std(header_deadline),
        client
            .get(url)
            .bearer_auth(credential.expose_secret())
            .header(header::ACCEPT, "application/json")
            .send(),
    )
    .await
    .map_err(|_| {
        failure(
            OpenRouterAccountIdentityFailureKind::Timeout,
            "OpenRouter current-key response-header deadline expired",
        )
    })?
    .map_err(|error| {
        if error.is_timeout() {
            failure(
                OpenRouterAccountIdentityFailureKind::Timeout,
                "OpenRouter current-key transport deadline expired",
            )
        } else {
            failure(
                OpenRouterAccountIdentityFailureKind::Transport,
                "OpenRouter current-key HTTP request failed",
            )
        }
    })?;

    // 다른 origin으로의 Bearer 전달뿐 아니라 응답 위치 재해석도 금지합니다.
    if response.status() != StatusCode::OK {
        return Err(failure(
            OpenRouterAccountIdentityFailureKind::HttpStatus,
            "OpenRouter current-key request did not return HTTP 200",
        ));
    }
    if !response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_media_type)
    {
        return Err(failure(
            OpenRouterAccountIdentityFailureKind::MediaType,
            "OpenRouter current-key response is not JSON",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(failure(
            OpenRouterAccountIdentityFailureKind::Limit,
            "OpenRouter current-key response exceeds 64 KiB",
        ));
    }

    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = absolute_deadline.min(body_progress + BODY_IDLE_TIMEOUT);
        let next = timeout_at(TokioInstant::from_std(body_deadline), chunks.next())
            .await
            .map_err(|_| {
                failure(
                    OpenRouterAccountIdentityFailureKind::Timeout,
                    "OpenRouter current-key response-body deadline expired",
                )
            })?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(failure(
                        OpenRouterAccountIdentityFailureKind::Limit,
                        "OpenRouter current-key response exceeds 64 KiB",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(failure(
                    OpenRouterAccountIdentityFailureKind::Transport,
                    "OpenRouter current-key response body could not be read",
                ));
            },
            None => return Ok(bytes),
        }
    }
}

fn is_json_media_type(value: &str) -> bool {
    let media_type = value.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("application/json")
        || media_type
            .to_ascii_lowercase()
            .strip_prefix("application/")
            .is_some_and(|subtype| subtype.ends_with("+json"))
}

#[derive(Deserialize)]
struct CurrentKeyEnvelope {
    data: CurrentKeyData,
}

#[derive(Deserialize)]
struct CurrentKeyData {
    creator_user_id: String,
    #[serde(deserialize_with = "required_option")]
    organization_id: Option<String>,
    workspace_id: String,
}

fn parse_current_key(bytes: &[u8]) -> Result<String, OpenRouterAccountIdentityError> {
    let response: CurrentKeyEnvelope = serde_json::from_slice(bytes).map_err(|_| {
        failure(
            OpenRouterAccountIdentityFailureKind::Protocol,
            "OpenRouter current-key response has no unique typed creator_user_id",
        )
    })?;
    let CurrentKeyData {
        creator_user_id,
        organization_id,
        workspace_id,
    } = response.data;
    if !valid_account_component(&creator_user_id)
        || organization_id
            .as_deref()
            .is_some_and(|id| !valid_account_component(id))
        || !valid_workspace_id(&workspace_id)
    {
        return Err(failure(
            OpenRouterAccountIdentityFailureKind::Protocol,
            "OpenRouter current-key account identity is invalid",
        ));
    }
    let organization = organization_id
        .as_ref()
        .map_or_else(|| "none".to_owned(), |id| format!("some:{id}"));
    Ok(format!(
        "openrouter-account/v1|creator={creator_user_id}|organization={organization}|workspace={}",
        workspace_id.to_ascii_lowercase()
    ))
}

fn valid_account_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ACCOUNT_COMPONENT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_workspace_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
mod tests;
