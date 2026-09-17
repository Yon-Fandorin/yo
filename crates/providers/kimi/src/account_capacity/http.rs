use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Url, header, redirect, retry};
use tokio::{
    runtime::Builder,
    time::{Instant, timeout_at},
};
use yo_core::{AccountCapacitySnapshot, ApiCredential};

use super::{
    decode::{
        parse_kimi_account_capacity_snapshot_with_plan, parse_kimi_account_plan,
        require_code_membership,
    },
    error::{
        KimiAccountCapacityError, KimiAccountCapacityFailureKind, failure, limit_failure,
        timeout_failure,
    },
    model::MAX_RESPONSE_BYTES,
};
use crate::catalog::KimiCatalogSeed;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn read_kimi_account_capacity(
    seed: &KimiCatalogSeed,
    credential: &ApiCredential,
) -> Result<AccountCapacitySnapshot, KimiAccountCapacityError> {
    require_code_membership(seed)?;
    let profile_url = seed.endpoint().append_path_segment("me").map_err(|_| {
        failure(
            KimiAccountCapacityFailureKind::Configuration,
            "Kimi Code endpoint cannot accept the account-profile path",
        )
    })?;
    let usage_url = seed.endpoint().append_path_segment("usages").map_err(|_| {
        failure(
            KimiAccountCapacityFailureKind::Configuration,
            "Kimi Code endpoint cannot accept the usages path",
        )
    })?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(retry::never())
        .build()
        .map_err(|_| {
            failure(
                KimiAccountCapacityFailureKind::Configuration,
                "cannot initialize the Kimi account-capacity HTTP client",
            )
        })?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            failure(
                KimiAccountCapacityFailureKind::Transport,
                "cannot initialize the Kimi account-capacity request runtime",
            )
        })?;
    let profile_bytes = runtime.block_on(fetch(&client, profile_url, credential))?;
    let plan = parse_kimi_account_plan(&profile_bytes)?;
    let usage_bytes = runtime.block_on(fetch(&client, usage_url, credential))?;
    parse_kimi_account_capacity_snapshot_with_plan(seed, &usage_bytes, Some(plan))
}

pub(super) async fn fetch(
    client: &Client,
    request_url: Url,
    credential: &ApiCredential,
) -> Result<Vec<u8>, KimiAccountCapacityError> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let response = timeout_at(
        deadline,
        client
            .get(request_url)
            .bearer_auth(credential.expose_secret())
            .header(header::ACCEPT, "application/json")
            .send(),
    )
    .await
    .map_err(|_| timeout_failure("Kimi account-capacity request deadline expired"))?
    .map_err(map_reqwest_error)?;
    if response.status().is_redirection() {
        return Err(failure(
            KimiAccountCapacityFailureKind::Transport,
            "Kimi account-capacity endpoint redirected",
        ));
    }
    if !response.status().is_success() {
        return Err(failure(
            KimiAccountCapacityFailureKind::HttpStatus,
            format!(
                "Kimi account-capacity endpoint returned HTTP status {}",
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
            KimiAccountCapacityFailureKind::MediaType,
            "Kimi account-capacity success did not return a JSON media type",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(limit_failure(
            "Kimi account-capacity response exceeds 1 MiB",
        ));
    }

    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = deadline.min(body_progress + BODY_IDLE_TIMEOUT);
        let next = timeout_at(body_deadline, chunks.next())
            .await
            .map_err(|_| timeout_failure("Kimi account-capacity response-body deadline expired"))?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(limit_failure(
                        "Kimi account-capacity response exceeds 1 MiB",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(failure(
                    KimiAccountCapacityFailureKind::Transport,
                    "Kimi account-capacity response body could not be read",
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

fn map_reqwest_error(error: reqwest::Error) -> KimiAccountCapacityError {
    if error.is_timeout() {
        timeout_failure("Kimi account-capacity transport deadline expired")
    } else {
        failure(
            KimiAccountCapacityFailureKind::Transport,
            "Kimi account-capacity HTTP request failed",
        )
    }
}
