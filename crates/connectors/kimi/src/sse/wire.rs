use super::{ConnectorError, ConnectorFailureKind, Value};

pub(super) fn string_at<'a>(
    value: &'a Value,
    field: &'static str,
    label: &'static str,
) -> Result<&'a str, ConnectorError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| protocol_failure(format!("Chat Completions {label} is missing")))
}

pub(super) fn encoded_json_string_payload_bytes(value: &str) -> Result<usize, ConnectorError> {
    serde_json::to_string(value)
        .map(|encoded| encoded.len() - 2)
        .map_err(|_| protocol_failure("Kimi private string cannot be encoded"))
}

pub(super) fn valid_kimi_function_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    (3..=64).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(super) fn unsigned_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<u64, ConnectorError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol_failure(format!("Chat Completions {label} is not unsigned")))
}

pub(super) fn optional_string<'a>(
    value: Option<&'a Value>,
    label: &'static str,
) -> Result<Option<&'a str>, ConnectorError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(protocol_failure(format!(
            "Chat Completions {label} is not a string or null"
        ))),
    }
}

pub(super) fn bounded_sum(
    current: usize,
    added: usize,
    limit: usize,
    message: &'static str,
) -> Result<usize, ConnectorError> {
    let total = current
        .checked_add(added)
        .ok_or_else(|| limit_failure(message))?;
    if total > limit {
        Err(limit_failure(message))
    } else {
        Ok(total)
    }
}

pub(super) fn protocol_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Protocol, message)
}

pub(super) fn limit_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Limit, message)
}
