use serde::Deserialize;

use super::{
    AccountId, ApiDialect, ConnectorId, EffectiveModelBinding, EffectiveModelProfile, ModelId,
    ModelProfileLayer, ModelProfileParameters, ModelServiceError, NormalizedEndpoint, ProviderId,
    VersionedProfileId,
};

/// A model binding together with every resolved behavior field that defines its epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompleteModelBinding {
    binding: EffectiveModelBinding,
    profile: EffectiveModelProfile,
}

impl CompleteModelBinding {
    pub fn new(
        binding: EffectiveModelBinding,
        profile: EffectiveModelProfile,
    ) -> Result<Self, ModelServiceError> {
        if binding.api_dialect() != profile.api_dialect() {
            return Err(ModelServiceError::new(
                "effective binding api_dialect does not match its resolved profile",
            ));
        }
        Ok(Self { binding, profile })
    }

    /// Decodes the closed `yo.complete-model-binding/v1` value without retyping numbers.
    pub fn from_durable_json(value: &str) -> Result<Self, ModelServiceError> {
        validate_json_number_spellings(value)?;
        let durable: DurableCompleteBinding = serde_json::from_str(value).map_err(|error| {
            ModelServiceError::new(format!(
                "durable complete model binding is malformed: {error}"
            ))
        })?;
        let DurableCompleteBinding {
            provider,
            account,
            model,
            connector,
            base_url,
            api_dialect,
            tokenizer_profile,
            input_token_limit,
            max_output_tokens,
            reasoning_parameters,
            optional_request_parameters,
            tool_capability_policy,
            replay_profile,
        } = durable;

        let dialect = api_dialect.parse::<ApiDialect>()?;
        let binding = EffectiveModelBinding::from_durable(
            ProviderId::new(provider)?,
            AccountId::new(account)?,
            ModelId::new(model)?,
            ConnectorId::new(connector)?,
            dialect,
            NormalizedEndpoint::parse(&base_url)?,
        )?;
        let layer = ModelProfileLayer::new(
            Some(dialect),
            Some(VersionedProfileId::new(tokenizer_profile)?),
            Some(input_token_limit),
            max_output_tokens,
            Some(reasoning_parameters),
            Some(optional_request_parameters),
            Some(VersionedProfileId::new(tool_capability_policy)?),
        )
        .with_replay_profile(replay_profile.map(VersionedProfileId::new).transpose()?);
        let profile = EffectiveModelProfile::resolve(None, &layer)?;
        Self::new(binding, profile)
    }

    #[must_use]
    pub const fn binding(&self) -> &EffectiveModelBinding {
        &self.binding
    }

    #[must_use]
    pub const fn profile(&self) -> &EffectiveModelProfile {
        &self.profile
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableCompleteBinding {
    provider: String,
    account: String,
    model: String,
    connector: String,
    base_url: String,
    api_dialect: String,
    tokenizer_profile: String,
    input_token_limit: u64,
    #[serde(default, deserialize_with = "deserialize_optional_non_null_u64")]
    max_output_tokens: Option<u64>,
    reasoning_parameters: ModelProfileParameters,
    optional_request_parameters: ModelProfileParameters,
    tool_capability_policy: String,
    #[serde(default, deserialize_with = "deserialize_optional_non_null_string")]
    replay_profile: Option<String>,
}

fn deserialize_optional_non_null_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if matches!(
        value.as_str(),
        crate::SEMANTIC_REPLAY_PROFILE | crate::KIMI_PRIVATE_REPLAY_PROFILE
    ) {
        Ok(Some(value))
    } else {
        Err(serde::de::Error::custom(
            "present replay_profile is outside the closed supported set",
        ))
    }
}

fn deserialize_optional_non_null_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    u64::deserialize(deserializer).map(Some)
}

fn validate_json_number_spellings(value: &str) -> Result<(), ModelServiceError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte != b'-' && !byte.is_ascii_digit() {
            index += 1;
            continue;
        }
        let end = bytes[index..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b',' | b']' | b'}'))
            .map_or(bytes.len(), |offset| index + offset);
        let token = &value[index..end];
        let valid = if token.contains(['.', 'e', 'E']) {
            token.parse::<f64>().is_ok_and(f64::is_finite)
        } else if token.starts_with('-') {
            token.parse::<i64>().is_ok()
        } else {
            token.parse::<u64>().is_ok()
        };
        if !valid {
            return Err(ModelServiceError::new(
                "durable complete model binding has an out-of-range number",
            ));
        }
        index = end;
    }
    Ok(())
}
