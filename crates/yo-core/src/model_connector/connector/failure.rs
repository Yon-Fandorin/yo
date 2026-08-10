use std::{
    future::Future,
    time::{Duration, Instant},
};

use reqwest::StatusCode;

use super::{
    super::{ConnectorError, ConnectorFailureKind},
    ResponsesCancellation,
};

pub(super) fn configuration_failure(message: impl Into<String>) -> ConnectorError {
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
    ConnectorError::new(
        ConnectorFailureKind::HttpStatus,
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

pub(super) fn effective_timeout(
    started: Instant,
    total: Duration,
    phase: Duration,
    phase_message: &'static str,
) -> Result<EffectiveTimeout, ConnectorError> {
    let remaining = total.checked_sub(started.elapsed()).ok_or_else(|| {
        ConnectorError::new(
            ConnectorFailureKind::Timeout,
            "model-connector total request deadline expired",
        )
    })?;
    if remaining.is_zero() {
        return Err(ConnectorError::new(
            ConnectorFailureKind::Timeout,
            "model-connector total request deadline expired",
        ));
    }
    if remaining <= phase {
        Ok(EffectiveTimeout {
            duration: remaining,
            message: "model-connector total request deadline expired",
        })
    } else {
        Ok(EffectiveTimeout {
            duration: phase,
            message: phase_message,
        })
    }
}

pub(super) async fn cancellable_timeout<F, T>(
    cancellation: &ResponsesCancellation,
    timeout: EffectiveTimeout,
    future: F,
) -> Result<T, ConnectorError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        () = cancellation.0.cancelled() => Err(cancelled_failure()),
        result = tokio::time::timeout(timeout.duration, future) => {
            result.map_err(|_| ConnectorError::new(ConnectorFailureKind::Timeout, timeout.message))
        },
    }
}
