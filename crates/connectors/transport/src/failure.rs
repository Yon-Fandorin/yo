use std::{
    future::Future,
    time::{Duration, Instant},
};

use reqwest::StatusCode;
use yo_core::{ConnectorError, ConnectorFailureKind, ModelConnectorCancellation};

pub fn configuration_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Configuration, message)
}

pub(super) fn transport_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Transport, message)
}

pub(super) fn protocol_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Protocol, message)
}

pub(super) fn limit_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Limit, message)
}

pub(super) fn cancelled_failure() -> ConnectorError {
    ConnectorError::new(
        ConnectorFailureKind::Cancelled,
        "model-connector request was cancelled",
    )
}

pub(super) fn http_status_failure(status: StatusCode) -> ConnectorError {
    let request_failure = match status.as_u16() {
        401 => yo_core::ModelRequestFailureKind::Authentication,
        403 => yo_core::ModelRequestFailureKind::AccessDenied,
        408 => yo_core::ModelRequestFailureKind::Timeout,
        429 => yo_core::ModelRequestFailureKind::RateLimited,
        400..=499 => yo_core::ModelRequestFailureKind::RequestRejected,
        500..=599 => yo_core::ModelRequestFailureKind::ProviderUnavailable,
        _ => yo_core::ModelRequestFailureKind::Protocol,
    };
    ConnectorError::with_request_failure(
        ConnectorFailureKind::HttpStatus,
        request_failure,
        format!(
            "model-connector HTTP request returned status {}",
            status.as_u16()
        ),
    )
}

pub(super) fn map_reqwest_error(error: reqwest::Error) -> ConnectorError {
    if error.is_timeout() {
        ConnectorError::new(
            ConnectorFailureKind::Timeout,
            "model-connector HTTP transport deadline expired",
        )
    } else if error.is_redirect() {
        transport_failure("model-connector HTTP redirect was rejected")
    } else {
        transport_failure("model-connector HTTP request failed")
    }
}

#[derive(Clone, Copy)]
pub(super) struct EffectiveTimeout {
    pub(super) duration: Duration,
    pub(super) message: &'static str,
}

pub(super) fn phase_timeout(
    request_started: Instant,
    absolute_request_timeout: Option<Duration>,
    phase_started: Instant,
    phase_timeout: Duration,
    phase_message: &'static str,
) -> Result<EffectiveTimeout, ConnectorError> {
    let now = Instant::now();
    let phase_remaining = phase_timeout
        .checked_sub(now.saturating_duration_since(phase_started))
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| ConnectorError::new(ConnectorFailureKind::Timeout, phase_message))?;
    let Some(absolute_request_timeout) = absolute_request_timeout else {
        return Ok(EffectiveTimeout {
            duration: phase_remaining,
            message: phase_message,
        });
    };
    let absolute_remaining = absolute_request_timeout
        .checked_sub(now.saturating_duration_since(request_started))
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            ConnectorError::new(
                ConnectorFailureKind::Timeout,
                "model-connector absolute request deadline expired",
            )
        })?;
    if absolute_remaining <= phase_remaining {
        Ok(EffectiveTimeout {
            duration: absolute_remaining,
            message: "model-connector absolute request deadline expired",
        })
    } else {
        Ok(EffectiveTimeout {
            duration: phase_remaining,
            message: phase_message,
        })
    }
}

pub(super) fn record_body_progress(
    phase_started: &mut Instant,
    bytes: &[u8],
    observed_at: Instant,
) {
    if !bytes.is_empty() {
        *phase_started = observed_at;
    }
}

pub(super) async fn cancellable_timeout<F, T>(
    cancellation: &ModelConnectorCancellation,
    timeout: EffectiveTimeout,
    future: F,
) -> Result<T, ConnectorError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(cancelled_failure()),
        result = tokio::time::timeout(timeout.duration, future) => {
            result.map_err(|_| ConnectorError::new(ConnectorFailureKind::Timeout, timeout.message))
        },
    }
}
