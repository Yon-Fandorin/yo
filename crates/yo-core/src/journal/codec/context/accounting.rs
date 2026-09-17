use serde_json::Value;

const MAX_CONTEXT_USAGE_ID_BYTES: usize = 256;
const MAX_CONTEXT_USAGE_ENDPOINT_BYTES: usize = 2_048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextSummaryUsage(Value);

impl ContextSummaryUsage {
    pub(crate) fn try_new(value: Value) -> Result<Self, &'static str> {
        validate_summary_usage(&value)?;
        Ok(Self(value))
    }

    pub(crate) const fn value(&self) -> &Value {
        &self.0
    }
}

fn validate_summary_usage(value: &Value) -> Result<(), &'static str> {
    let object = closed_object(
        value,
        &[
            "schema",
            "response_id",
            "round",
            "provider",
            "account",
            "model",
            "connector",
            "api_dialect",
            "base_url",
            "usage",
            "cache_read_input_tokens",
        ],
    )?;
    if object.get("schema").and_then(Value::as_str) != Some("yo.model-usage-receipt/v1")
        || !object
            .get("round")
            .and_then(Value::as_u64)
            .is_some_and(|round| round > 0)
        || !object
            .get("response_id")
            .and_then(Value::as_str)
            .is_some_and(|value| valid_usage_attribution(value, MAX_CONTEXT_USAGE_ID_BYTES))
        || !object
            .get("provider")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::ProviderId::new(value).is_ok())
        || !object
            .get("account")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::AccountId::new(value).is_ok())
        || !object
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::ModelId::new(value).is_ok())
        || !object
            .get("connector")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::ConnectorId::new(value).is_ok())
        || !object
            .get("api_dialect")
            .and_then(Value::as_str)
            .is_some_and(|value| value.parse::<crate::ApiDialect>().is_ok())
        || !object
            .get("base_url")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                value.len() <= MAX_CONTEXT_USAGE_ENDPOINT_BYTES
                    && crate::NormalizedEndpoint::parse(value)
                        .is_ok_and(|endpoint| endpoint.as_str() == value)
            })
    {
        return Err("context summary usage source is invalid");
    }
    let usage = object
        .get("usage")
        .ok_or("context summary usage is missing usage")?;
    let usage = closed_object(
        usage,
        &[
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "reasoning_tokens",
        ],
    )?;
    let token = |field| {
        usage
            .get(field)
            .and_then(Value::as_u64)
            .ok_or("context summary usage token value is invalid")
    };
    let input_tokens = token("input_tokens")?;
    let output_tokens = token("output_tokens")?;
    let total_tokens = token("total_tokens")?;
    let reasoning_tokens = match usage.get("reasoning_tokens") {
        Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or("context summary reasoning token value is invalid")?,
        ),
        None => return Err("context summary usage is missing reasoning tokens"),
    };
    if input_tokens.checked_add(output_tokens) != Some(total_tokens)
        || reasoning_tokens.is_some_and(|tokens| tokens > output_tokens)
    {
        return Err("context summary usage token relationship is invalid");
    }
    let cache = object
        .get("cache_read_input_tokens")
        .ok_or("context summary usage is missing cache availability")?;
    let cache = cache
        .as_object()
        .ok_or("context summary cache availability is invalid")?;
    match cache.get("availability").and_then(Value::as_str) {
        Some("reported") => {
            closed_map(cache, &["availability", "tokens", "source_profile"])?;
            if !cache
                .get("tokens")
                .and_then(Value::as_u64)
                .is_some_and(|tokens| tokens <= input_tokens)
                || !cache
                    .get("source_profile")
                    .and_then(Value::as_str)
                    .is_some_and(|value| crate::VersionedProfileId::new(value).is_ok())
            {
                return Err("reported context summary cache usage is invalid");
            }
        },
        Some("absent") => {
            closed_map(cache, &["availability", "source_profile"])?;
            if !cache
                .get("source_profile")
                .and_then(Value::as_str)
                .is_some_and(|value| crate::VersionedProfileId::new(value).is_ok())
            {
                return Err("absent context summary cache source is invalid");
            }
        },
        Some("unsupported") => closed_map(cache, &["availability"])?,
        _ => return Err("context summary cache availability is unsupported"),
    }
    Ok(())
}

fn valid_usage_attribution(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn closed_object<'a>(
    value: &'a Value,
    fields: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, &'static str> {
    let object = value.as_object().ok_or("context value must be an object")?;
    closed_map(object, fields)?;
    Ok(object)
}

fn closed_map(
    object: &serde_json::Map<String, Value>,
    fields: &[&str],
) -> Result<(), &'static str> {
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err("context value contains an unknown field");
    }
    Ok(())
}
