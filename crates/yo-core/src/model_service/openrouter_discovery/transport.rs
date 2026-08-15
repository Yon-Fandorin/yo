use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url, header, redirect};
use tokio::time::{Instant as TokioInstant, timeout_at};

use super::{
    OpenRouterDiscoveryError, OpenRouterDiscoveryFailureKind, failure, limit_failure,
    timeout_failure,
};
use crate::{ApiCredential, NormalizedEndpoint};

pub(super) const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_REDIRECTS: usize = 3;
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Copy)]
pub(super) struct DiscoveryTimeouts {
    pub(super) response_header: Duration,
    pub(super) body_idle: Duration,
    pub(super) absolute: Duration,
}

pub(super) const DISCOVERY_TIMEOUTS: DiscoveryTimeouts = DiscoveryTimeouts {
    response_header: RESPONSE_HEADER_TIMEOUT,
    body_idle: BODY_IDLE_TIMEOUT,
    absolute: ABSOLUTE_TIMEOUT,
};

pub(super) fn fetch_catalog(
    endpoint: &NormalizedEndpoint,
    credential: &ApiCredential,
) -> Result<Vec<u8>, OpenRouterDiscoveryError> {
    let request_url = discovery_url(endpoint)?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| {
            failure(
                OpenRouterDiscoveryFailureKind::Configuration,
                "cannot initialize the OpenRouter discovery HTTP client",
            )
        })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            failure(
                OpenRouterDiscoveryFailureKind::Transport,
                "cannot initialize the OpenRouter discovery request runtime",
            )
        })?;
    runtime.block_on(fetch_catalog_with_timeouts(
        &client,
        request_url,
        credential,
        DISCOVERY_TIMEOUTS,
    ))
}

pub(super) async fn fetch_catalog_with_timeouts(
    client: &Client,
    request_url: Url,
    credential: &ApiCredential,
    timeouts: DiscoveryTimeouts,
) -> Result<Vec<u8>, OpenRouterDiscoveryError> {
    let request_started = Instant::now();
    let absolute_deadline = request_started + timeouts.absolute;
    let origin = Origin::new(&request_url)?;
    let mut attempt_url = request_url;
    let mut followed_redirects = 0_usize;
    let response = loop {
        let header_deadline =
            deadline(absolute_deadline, Instant::now() + timeouts.response_header);
        let send = client
            .get(attempt_url.clone())
            .bearer_auth(credential.expose_secret())
            .header(header::ACCEPT, "application/json")
            .send();
        let response = timeout_at(TokioInstant::from_std(header_deadline), send)
            .await
            .map_err(|_| timeout_failure("OpenRouter discovery response-header deadline expired"))?
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
                OpenRouterDiscoveryFailureKind::Transport,
                "OpenRouter discovery redirect limit exceeded",
            ));
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                failure(
                    OpenRouterDiscoveryFailureKind::Transport,
                    "OpenRouter discovery redirect has no valid location",
                )
            })?;
        let target = attempt_url.join(location).map_err(|_| {
            failure(
                OpenRouterDiscoveryFailureKind::Transport,
                "OpenRouter discovery redirect location is invalid",
            )
        })?;
        if !origin.matches(&target) {
            return Err(failure(
                OpenRouterDiscoveryFailureKind::Transport,
                "OpenRouter discovery redirect changed origin",
            ));
        }
        followed_redirects += 1;
        attempt_url = target;
    };

    if !response.status().is_success() {
        return Err(failure(
            OpenRouterDiscoveryFailureKind::HttpStatus,
            format!(
                "OpenRouter discovery returned HTTP status {}",
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
            OpenRouterDiscoveryFailureKind::MediaType,
            "OpenRouter discovery success did not return a JSON media type",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(limit_failure("OpenRouter discovery response exceeds 8 MiB"));
    }

    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = deadline(absolute_deadline, body_progress + timeouts.body_idle);
        let next = timeout_at(TokioInstant::from_std(body_deadline), chunks.next())
            .await
            .map_err(|_| timeout_failure("OpenRouter discovery response-body deadline expired"))?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(limit_failure("OpenRouter discovery response exceeds 8 MiB"));
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(failure(
                    OpenRouterDiscoveryFailureKind::Transport,
                    "OpenRouter discovery response body could not be read",
                ));
            },
            None => return Ok(bytes),
        }
    }
}

pub(super) fn discovery_url(
    endpoint: &NormalizedEndpoint,
) -> Result<Url, OpenRouterDiscoveryError> {
    endpoint
        .append_path_segments(&["models", "user"])
        .map_err(|_| {
            failure(
                OpenRouterDiscoveryFailureKind::Configuration,
                "OpenRouter discovery endpoint cannot accept path segments",
            )
        })
}

pub(super) fn is_json_media_type(value: &str) -> bool {
    let media_type = value.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("application/json")
        || media_type
            .to_ascii_lowercase()
            .strip_prefix("application/")
            .is_some_and(|subtype| subtype.ends_with("+json"))
}

fn deadline(left: Instant, right: Instant) -> Instant {
    left.min(right)
}

fn map_reqwest_error(error: reqwest::Error) -> OpenRouterDiscoveryError {
    if error.is_timeout() {
        timeout_failure("OpenRouter discovery transport deadline expired")
    } else {
        failure(
            OpenRouterDiscoveryFailureKind::Transport,
            "OpenRouter discovery HTTP request failed",
        )
    }
}

#[derive(Clone)]
pub(super) struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    pub(super) fn new(url: &Url) -> Result<Self, OpenRouterDiscoveryError> {
        let host = url.host_str().ok_or_else(|| {
            failure(
                OpenRouterDiscoveryFailureKind::Configuration,
                "OpenRouter discovery URL has no host",
            )
        })?;
        let port = url.port_or_known_default().ok_or_else(|| {
            failure(
                OpenRouterDiscoveryFailureKind::Configuration,
                "OpenRouter discovery URL has no effective port",
            )
        })?;
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host: host.to_owned(),
            port,
        })
    }

    pub(super) fn matches(&self, url: &Url) -> bool {
        url.scheme() == self.scheme
            && url.host_str() == Some(self.host.as_str())
            && url.port_or_known_default() == Some(self.port)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    }
}
