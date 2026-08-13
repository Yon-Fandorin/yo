use std::{fmt, fmt::Write as _, str::FromStr};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{ApiDialect, ConnectorId, ModelServiceError, NormalizedEndpoint};

const BINDING_PROFILE_V1_DOMAIN: &[u8] = b"yo.binding-profile/v1\0";
const MAX_VERSIONED_PROFILE_ID_BYTES: usize = 128;

const NORMALIZED_ENDPOINT_FIELD: &[u8] = b"normalized_endpoint";
const API_DIALECT_FIELD: &[u8] = b"api_dialect";
const RESOLVED_CONNECTOR_ID_FIELD: &[u8] = b"resolved_connector_id";
const TOKENIZER_PROFILE_FIELD: &[u8] = b"tokenizer_profile";
const CONTEXT_LIMIT_FIELD: &[u8] = b"context_limit";
const OUTPUT_LIMIT_FIELD: &[u8] = b"output_limit";
const REASONING_PARAMETERS_FIELD: &[u8] = b"reasoning_parameters";
const OPTIONAL_REQUEST_PARAMETERS_FIELD: &[u8] = b"optional_request_parameters";
const TOOL_CAPABILITY_POLICY_FIELD: &[u8] = b"tool_capability_policy";
const VERIFICATION_PROFILE_FIELD: &[u8] = b"verification_profile";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BindingProfileSchema {
    V1,
}

impl BindingProfileSchema {
    pub const V1_ID: &'static str = "yo.binding-profile/v1";

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => Self::V1_ID,
        }
    }
}

impl fmt::Display for BindingProfileSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BindingProfileDigest(String);

impl BindingProfileDigest {
    fn from_canonical_bytes(bytes: &[u8]) -> Self {
        let mut value = String::with_capacity("sha256:".len() + 64);
        value.push_str("sha256:");
        for byte in Sha256::digest(bytes) {
            let _ = write!(value, "{byte:02x}");
        }
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BindingProfileDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for BindingProfileDigest {
    type Err = ModelServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ModelServiceError::new(
                "binding profile digest must start with sha256:",
            ));
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ModelServiceError::new(
                "binding profile digest must contain exactly 64 lowercase hexadecimal digits after sha256:",
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalJson {
    value: Value,
    bytes: Vec<u8>,
}

impl CanonicalJson {
    fn new(label: &'static str, value: Value) -> Result<Self, ModelServiceError> {
        let bytes = serde_json_canonicalizer::to_vec(&value).map_err(|error| {
            ModelServiceError::new(format!(
                "binding profile {label} is not RFC 8785 canonicalizable: {error}"
            ))
        })?;
        Ok(Self { value, bytes })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingProfileV1 {
    normalized_endpoint: NormalizedEndpoint,
    api_dialect: ApiDialect,
    resolved_connector_id: ConnectorId,
    tokenizer_profile: String,
    context_limit: u64,
    output_limit: u64,
    reasoning_parameters: CanonicalJson,
    optional_request_parameters: CanonicalJson,
    tool_capability_policy: String,
    verification_profile: String,
    canonical_bytes: Vec<u8>,
    digest: BindingProfileDigest,
}

impl BindingProfileV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        normalized_endpoint: NormalizedEndpoint,
        api_dialect: ApiDialect,
        resolved_connector_id: ConnectorId,
        tokenizer_profile: impl Into<String>,
        context_limit: u64,
        output_limit: u64,
        reasoning_parameters: Value,
        optional_request_parameters: Value,
        tool_capability_policy: impl Into<String>,
        verification_profile: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let expected_connector_id = ConnectorId::for_dialect(api_dialect);
        if resolved_connector_id != expected_connector_id {
            return Err(ModelServiceError::new(format!(
                "binding profile connector {} does not match api_dialect {}",
                resolved_connector_id, api_dialect
            )));
        }
        let tokenizer_profile = tokenizer_profile.into();
        let tool_capability_policy = tool_capability_policy.into();
        let verification_profile = verification_profile.into();
        validate_versioned_profile_id("tokenizer_profile", &tokenizer_profile)?;
        validate_versioned_profile_id("tool_capability_policy", &tool_capability_policy)?;
        validate_versioned_profile_id("verification_profile", &verification_profile)?;

        let reasoning_parameters =
            CanonicalJson::new("reasoning_parameters", reasoning_parameters)?;
        let optional_request_parameters =
            CanonicalJson::new("optional_request_parameters", optional_request_parameters)?;

        let mut canonical_bytes = BINDING_PROFILE_V1_DOMAIN.to_vec();
        append_field(
            &mut canonical_bytes,
            NORMALIZED_ENDPOINT_FIELD,
            normalized_endpoint.as_str().as_bytes(),
        )?;
        append_field(
            &mut canonical_bytes,
            API_DIALECT_FIELD,
            api_dialect.as_str().as_bytes(),
        )?;
        append_field(
            &mut canonical_bytes,
            RESOLVED_CONNECTOR_ID_FIELD,
            resolved_connector_id.as_str().as_bytes(),
        )?;
        append_field(
            &mut canonical_bytes,
            TOKENIZER_PROFILE_FIELD,
            tokenizer_profile.as_bytes(),
        )?;

        let context_limit_bytes = context_limit.to_string();
        append_field(
            &mut canonical_bytes,
            CONTEXT_LIMIT_FIELD,
            context_limit_bytes.as_bytes(),
        )?;
        let output_limit_bytes = output_limit.to_string();
        append_field(
            &mut canonical_bytes,
            OUTPUT_LIMIT_FIELD,
            output_limit_bytes.as_bytes(),
        )?;
        append_field(
            &mut canonical_bytes,
            REASONING_PARAMETERS_FIELD,
            &reasoning_parameters.bytes,
        )?;
        append_field(
            &mut canonical_bytes,
            OPTIONAL_REQUEST_PARAMETERS_FIELD,
            &optional_request_parameters.bytes,
        )?;
        append_field(
            &mut canonical_bytes,
            TOOL_CAPABILITY_POLICY_FIELD,
            tool_capability_policy.as_bytes(),
        )?;
        append_field(
            &mut canonical_bytes,
            VERIFICATION_PROFILE_FIELD,
            verification_profile.as_bytes(),
        )?;

        let digest = BindingProfileDigest::from_canonical_bytes(&canonical_bytes);
        Ok(Self {
            normalized_endpoint,
            api_dialect,
            resolved_connector_id,
            tokenizer_profile,
            context_limit,
            output_limit,
            reasoning_parameters,
            optional_request_parameters,
            tool_capability_policy,
            verification_profile,
            canonical_bytes,
            digest,
        })
    }

    #[must_use]
    pub const fn schema(&self) -> BindingProfileSchema {
        BindingProfileSchema::V1
    }

    #[must_use]
    pub const fn normalized_endpoint(&self) -> &NormalizedEndpoint {
        &self.normalized_endpoint
    }

    #[must_use]
    pub const fn api_dialect(&self) -> ApiDialect {
        self.api_dialect
    }

    #[must_use]
    pub const fn resolved_connector_id(&self) -> &ConnectorId {
        &self.resolved_connector_id
    }

    #[must_use]
    pub fn tokenizer_profile(&self) -> &str {
        &self.tokenizer_profile
    }

    #[must_use]
    pub const fn context_limit(&self) -> u64 {
        self.context_limit
    }

    #[must_use]
    pub const fn output_limit(&self) -> u64 {
        self.output_limit
    }

    #[must_use]
    pub const fn reasoning_parameters(&self) -> &Value {
        &self.reasoning_parameters.value
    }

    #[must_use]
    pub const fn optional_request_parameters(&self) -> &Value {
        &self.optional_request_parameters.value
    }

    #[must_use]
    pub fn tool_capability_policy(&self) -> &str {
        &self.tool_capability_policy
    }

    #[must_use]
    pub fn verification_profile(&self) -> &str {
        &self.verification_profile
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    #[must_use]
    pub const fn digest(&self) -> &BindingProfileDigest {
        &self.digest
    }
}

fn validate_versioned_profile_id(
    label: &'static str,
    value: &str,
) -> Result<(), ModelServiceError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_VERSIONED_PROFILE_ID_BYTES
        && value.is_ascii()
        && value.split_once("/v").is_some_and(|(name, version)| {
            let mut name_bytes = name.bytes();
            let name_valid = name_bytes.next().is_some_and(is_lower_ascii_or_digit)
                && name_bytes.all(|byte| {
                    is_lower_ascii_or_digit(byte) || matches!(byte, b'.' | b'_' | b'-')
                });
            let mut version_bytes = version.bytes();
            let version_valid = version_bytes
                .next()
                .is_some_and(|byte| matches!(byte, b'1'..=b'9'))
                && version_bytes.all(|byte| byte.is_ascii_digit());
            name_valid && version_valid
        });

    if !valid {
        return Err(ModelServiceError::new(format!(
            "binding profile {label} must match yo.versioned-profile-id/v1: 1 to {MAX_VERSIONED_PROFILE_ID_BYTES} ASCII bytes with name [a-z0-9][a-z0-9._-]* and version /v[1-9][0-9]*"
        )));
    }
    Ok(())
}

const fn is_lower_ascii_or_digit(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

fn append_field(output: &mut Vec<u8>, name: &[u8], value: &[u8]) -> Result<(), ModelServiceError> {
    let name_length = u64::try_from(name.len())
        .map_err(|_| ModelServiceError::new("binding profile field name exceeds u64 framing"))?;
    let value_length = u64::try_from(value.len())
        .map_err(|_| ModelServiceError::new("binding profile field value exceeds u64 framing"))?;
    output.extend_from_slice(&name_length.to_be_bytes());
    output.extend_from_slice(name);
    output.extend_from_slice(&value_length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}
