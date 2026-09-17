use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::ModelServiceError;

/// 실제 model request failure 하나를 secret 없이 분류한 닫힌 집합입니다.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelRequestFailureKind {
    Authentication,
    AccessDenied,
    ModelUnavailable,
    RateLimited,
    RequestRejected,
    ProviderUnavailable,
    Transport,
    Timeout,
    Protocol,
    ResponseLimit,
    LocalConfiguration,
}

impl ModelRequestFailureKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::AccessDenied => "access_denied",
            Self::ModelUnavailable => "model_unavailable",
            Self::RateLimited => "rate_limited",
            Self::RequestRejected => "request_rejected",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::Protocol => "protocol",
            Self::ResponseLimit => "response_limit",
            Self::LocalConfiguration => "local_configuration",
        }
    }

    pub(in super::super::super) fn parse(value: &str) -> Option<Self> {
        match value {
            "authentication" => Some(Self::Authentication),
            "access_denied" => Some(Self::AccessDenied),
            "model_unavailable" => Some(Self::ModelUnavailable),
            "rate_limited" => Some(Self::RateLimited),
            "request_rejected" => Some(Self::RequestRejected),
            "provider_unavailable" => Some(Self::ProviderUnavailable),
            "transport" => Some(Self::Transport),
            "timeout" => Some(Self::Timeout),
            "protocol" => Some(Self::Protocol),
            "response_limit" => Some(Self::ResponseLimit),
            "local_configuration" => Some(Self::LocalConfiguration),
            _ => None,
        }
    }
}

impl Display for ModelRequestFailureKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str(self.as_str())
    }
}

/// complete-binding identity와 분리해 보존하는 model별 warning-only 관찰입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelLastFailure {
    kind: ModelRequestFailureKind,
    observed_at: String,
}

impl ModelLastFailure {
    pub fn new(
        kind: ModelRequestFailureKind,
        observed_at: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let observed_at = observed_at.into();
        let timestamp = observed_at.parse::<jiff::Timestamp>().map_err(|_| {
            ModelServiceError::new("model last_failure observed_at must be canonical UTC RFC 3339")
        })?;
        if timestamp.subsec_nanosecond() != 0 || timestamp.to_string() != observed_at {
            return Err(ModelServiceError::new(
                "model last_failure observed_at must be canonical UTC RFC 3339 at whole-second precision",
            ));
        }
        Ok(Self { kind, observed_at })
    }

    #[must_use]
    pub const fn kind(&self) -> ModelRequestFailureKind {
        self.kind
    }

    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
}
