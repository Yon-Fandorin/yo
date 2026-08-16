use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url, header, redirect};
use tokio::time::{Instant as TokioInstant, timeout_at};

use super::{KimiCatalogError, KimiCatalogFailureKind, failure, limit_failure, timeout_failure};
use crate::{ApiCredential, NormalizedEndpoint};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(2 * 60);

pub(super) fn fetch_catalog(
    endpoint: &NormalizedEndpoint,
    credential: &ApiCredential,
) -> Result<Vec<u8>, KimiCatalogError> {
    let request_url = endpoint.append_path_segment("models").map_err(|_| {
        failure(
            KimiCatalogFailureKind::Configuration,
            "Kimi catalog endpoint cannot accept the models path",
        )
    })?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| {
            failure(
                KimiCatalogFailureKind::Configuration,
                "cannot initialize the Kimi catalog HTTP client",
            )
        })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            failure(
                KimiCatalogFailureKind::Transport,
                "cannot initialize the Kimi catalog request runtime",
            )
        })?;
    runtime.block_on(fetch(&client, request_url, credential))
}

pub(super) async fn fetch(
    client: &Client,
    request_url: Url,
    credential: &ApiCredential,
) -> Result<Vec<u8>, KimiCatalogError> {
    let absolute_deadline = Instant::now() + ABSOLUTE_TIMEOUT;
    let origin = Origin::new(&request_url)?;
    let mut attempt_url = request_url;
    let mut followed_redirects = 0_usize;
    let response = loop {
        let header_deadline = absolute_deadline.min(Instant::now() + RESPONSE_HEADER_TIMEOUT);
        let send = client
            .get(attempt_url.clone())
            .bearer_auth(credential.expose_secret())
            .header(header::ACCEPT, "application/json")
            .send();
        let response = timeout_at(TokioInstant::from_std(header_deadline), send)
            .await
            .map_err(|_| timeout_failure("Kimi catalog response-header deadline expired"))?
            .map_err(map_reqwest_error)?;
        if !matches!(
            response.status(),
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            break response;
        }
        if followed_redirects >= MAX_REDIRECTS {
            return Err(failure(
                KimiCatalogFailureKind::Transport,
                "Kimi catalog redirect limit exceeded",
            ));
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                failure(
                    KimiCatalogFailureKind::Transport,
                    "Kimi catalog redirect has no valid location",
                )
            })?;
        let target = attempt_url.join(location).map_err(|_| {
            failure(
                KimiCatalogFailureKind::Transport,
                "Kimi catalog redirect location is invalid",
            )
        })?;
        if !origin.matches(&target) {
            return Err(failure(
                KimiCatalogFailureKind::Transport,
                "Kimi catalog redirect changed origin",
            ));
        }
        followed_redirects += 1;
        attempt_url = target;
    };
    if !response.status().is_success() {
        return Err(failure(
            KimiCatalogFailureKind::HttpStatus,
            format!(
                "Kimi catalog returned HTTP status {}",
                response.status().as_u16()
            ),
        ));
    }
    if !response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_media_type)
    {
        return Err(failure(
            KimiCatalogFailureKind::MediaType,
            "Kimi catalog success did not return a JSON media type",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(limit_failure("Kimi catalog response exceeds 8 MiB"));
    }
    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = absolute_deadline.min(body_progress + BODY_IDLE_TIMEOUT);
        let next = timeout_at(TokioInstant::from_std(body_deadline), chunks.next())
            .await
            .map_err(|_| timeout_failure("Kimi catalog response-body deadline expired"))?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(limit_failure("Kimi catalog response exceeds 8 MiB"));
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(failure(
                    KimiCatalogFailureKind::Transport,
                    "Kimi catalog response body could not be read",
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

fn map_reqwest_error(error: reqwest::Error) -> KimiCatalogError {
    if error.is_timeout() {
        timeout_failure("Kimi catalog transport deadline expired")
    } else {
        failure(
            KimiCatalogFailureKind::Transport,
            "Kimi catalog HTTP request failed",
        )
    }
}

struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    fn new(url: &Url) -> Result<Self, KimiCatalogError> {
        let host = url.host_str().ok_or_else(|| {
            failure(
                KimiCatalogFailureKind::Configuration,
                "Kimi catalog URL has no host",
            )
        })?;
        let port = url.port_or_known_default().ok_or_else(|| {
            failure(
                KimiCatalogFailureKind::Configuration,
                "Kimi catalog URL has no effective port",
            )
        })?;
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host: host.to_owned(),
            port,
        })
    }

    fn matches(&self, url: &Url) -> bool {
        url.scheme() == self.scheme
            && url.host_str() == Some(self.host.as_str())
            && url.port_or_known_default() == Some(self.port)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    }
}
