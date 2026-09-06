use super::{
    CACHE_READ_SOURCE_PROFILE, CacheReadInputTokens, ConnectorError, ConnectorFailureKind,
    MessageContent, ModelConnectorUsage, TextPart, Value, VersionedProfileId,
};

pub(super) fn responses_incomplete_failure(
    reason: Option<&str>,
) -> yo_core::ModelRequestFailureKind {
    match reason {
        Some("max_output_tokens") => yo_core::ModelRequestFailureKind::ResponseLimit,
        Some("content_filter") => yo_core::ModelRequestFailureKind::RequestRejected,
        _ => yo_core::ModelRequestFailureKind::Protocol,
    }
}

pub(super) fn responses_failed_failure(code: Option<&str>) -> yo_core::ModelRequestFailureKind {
    match code {
        Some("invalid_api_key") => yo_core::ModelRequestFailureKind::Authentication,
        Some("insufficient_permissions") => yo_core::ModelRequestFailureKind::AccessDenied,
        Some("model_not_found") => yo_core::ModelRequestFailureKind::ModelUnavailable,
        Some("rate_limit_exceeded") => yo_core::ModelRequestFailureKind::RateLimited,
        Some("server_error") => yo_core::ModelRequestFailureKind::ProviderUnavailable,
        Some("request_timeout") => yo_core::ModelRequestFailureKind::Timeout,
        _ => yo_core::ModelRequestFailureKind::Protocol,
    }
}

impl MessageContent {
    pub(super) fn is_done(&self) -> bool {
        match self {
            Self::Text {
                stream_done,
                declared,
                part_done,
                ..
            }
            | Self::Refusal {
                stream_done,
                declared,
                part_done,
                ..
            } => *stream_done && (!*declared || *part_done),
        }
    }

    pub(super) fn value(&self) -> &str {
        match self {
            Self::Text { value, .. } | Self::Refusal { value, .. } => value,
        }
    }

    pub(super) fn stream_done(&self) -> bool {
        match self {
            Self::Text { stream_done, .. } | Self::Refusal { stream_done, .. } => *stream_done,
        }
    }

    pub(super) fn declared(&self) -> bool {
        match self {
            Self::Text { declared, .. } | Self::Refusal { declared, .. } => *declared,
        }
    }

    pub(super) fn part_done(&self) -> bool {
        match self {
            Self::Text { part_done, .. } | Self::Refusal { part_done, .. } => *part_done,
        }
    }

    pub(super) fn matches_type(&self, part_type: &str) -> bool {
        matches!(
            (self, part_type),
            (Self::Text { .. }, "output_text") | (Self::Refusal { .. }, "refusal")
        )
    }

    pub(super) fn mark_part_done(&mut self) {
        match self {
            Self::Text { part_done, .. } | Self::Refusal { part_done, .. } => *part_done = true,
        }
    }
}

impl TextPart {
    pub(super) fn implicit() -> Self {
        Self {
            value: String::new(),
            stream_done: false,
            declared: false,
            part_done: false,
        }
    }

    pub(super) fn declared(value: String) -> Self {
        Self {
            value,
            stream_done: false,
            declared: true,
            part_done: false,
        }
    }

    pub(super) fn is_done(&self) -> bool {
        self.stream_done && (!self.declared || self.part_done)
    }

    pub(super) fn finish_wrapper(&mut self, final_text: &str) -> Result<(), ConnectorError> {
        if !self.stream_done || !self.declared || self.part_done || self.value != final_text {
            return Err(protocol_failure(
                "final reasoning part disagrees with its accumulated stream",
            ));
        }
        self.part_done = true;
        Ok(())
    }
}

pub(super) fn value_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a Value, ConnectorError> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| protocol_failure(format!("Responses event is missing {label}")))?;
    }
    Ok(current)
}

pub(super) fn string_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a str, ConnectorError> {
    value_at(value, path, label)?
        .as_str()
        .ok_or_else(|| protocol_failure(format!("Responses {label} is not a string")))
}

pub(super) fn optional_string_at(
    value: &Value,
    path: &[&str],
) -> Result<Option<String>, ConnectorError> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Ok(None);
        };
        if next.is_null() {
            return Ok(None);
        }
        current = next;
    }
    current
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| protocol_failure("optional Responses field is not a string"))
}

pub(super) fn usize_at(
    value: &Value,
    path: &[&str],
    label: &'static str,
) -> Result<usize, ConnectorError> {
    let value = value_at(value, path, label)?
        .as_u64()
        .ok_or_else(|| protocol_failure(format!("Responses {label} is not an unsigned integer")))?;
    usize::try_from(value)
        .map_err(|_| protocol_failure(format!("Responses {label} is outside the host range")))
}

pub(super) fn usage_at(event: &Value) -> Result<ModelConnectorUsage, ConnectorError> {
    let response = value_at(event, &["response"], "response")?;
    let Some(usage) = response.get("usage") else {
        return Ok(ModelConnectorUsage {
            cache_read_input_tokens: cache_read_absent(),
            ..ModelConnectorUsage::default()
        });
    };
    if !usage.is_object() {
        return Err(protocol_failure("Responses usage is not an object"));
    }
    let input_tokens = optional_u64_at(usage, &["input_tokens"])?;
    Ok(ModelConnectorUsage {
        input_tokens,
        output_tokens: optional_u64_at(usage, &["output_tokens"])?,
        total_tokens: optional_u64_at(usage, &["total_tokens"])?,
        reasoning_tokens: optional_u64_at(usage, &["output_tokens_details", "reasoning_tokens"])?,
        cache_read_input_tokens: cache_read_input_tokens_at(usage, input_tokens)?,
    })
}

fn cache_read_input_tokens_at(
    usage: &Value,
    input_tokens: Option<u64>,
) -> Result<CacheReadInputTokens, ConnectorError> {
    let Some(details) = usage.get("input_tokens_details") else {
        return Ok(cache_read_absent());
    };
    if !details.is_object() {
        return Err(protocol_failure(
            "Responses input_tokens_details is not an object",
        ));
    }
    let Some(cached_tokens) = details.get("cached_tokens") else {
        return Ok(cache_read_absent());
    };
    let tokens = cached_tokens
        .as_u64()
        .ok_or_else(|| protocol_failure("Responses cached_tokens is not an unsigned integer"))?;
    let input_tokens = input_tokens.ok_or_else(|| {
        protocol_failure("Responses cached_tokens requires unsigned input_tokens")
    })?;
    if tokens > input_tokens {
        return Err(protocol_failure(
            "Responses cached_tokens exceeds input_tokens",
        ));
    }
    Ok(CacheReadInputTokens::Reported {
        tokens,
        source_profile: cache_read_source_profile(),
    })
}

fn cache_read_absent() -> CacheReadInputTokens {
    CacheReadInputTokens::Absent {
        source_profile: cache_read_source_profile(),
    }
}

fn cache_read_source_profile() -> VersionedProfileId {
    VersionedProfileId::new(CACHE_READ_SOURCE_PROFILE)
        .expect("the closed Responses usage source profile is valid")
}

fn optional_u64_at(value: &Value, path: &[&str]) -> Result<Option<u64>, ConnectorError> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Ok(None);
        };
        if next.is_null() {
            return Ok(None);
        }
        current = next;
    }
    current
        .as_u64()
        .map(Some)
        .ok_or_else(|| protocol_failure("Responses usage field is not an unsigned integer"))
}

pub(super) fn protocol_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Protocol, message)
}

pub(super) fn limit_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Limit, message)
}
