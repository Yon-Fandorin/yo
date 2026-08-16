use std::{collections::BTreeMap, fmt};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};

use super::{ApiDialect, ModelContextProfile, ModelServiceError};

const MAX_PROFILE_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct VersionedProfileId(String);

impl VersionedProfileId {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelServiceError> {
        let value = value.into();
        if !valid_versioned_profile_id(&value) {
            return Err(ModelServiceError::new(format!(
                "profile identifier {value:?} must match [a-z0-9][a-z0-9._-]*/v[1-9][0-9]* in 1 to {MAX_PROFILE_ID_BYTES} ASCII bytes"
            )));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionedProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn valid_versioned_profile_id(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_PROFILE_ID_BYTES || !value.is_ascii() {
        return false;
    }
    let Some((name, version)) = value.rsplit_once("/v") else {
        return false;
    };
    let mut name_bytes = name.bytes();
    let Some(first) = name_bytes.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    if !name_bytes.all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    }) {
        return false;
    }
    let mut version_bytes = version.bytes();
    matches!(version_bytes.next(), Some(b'1'..=b'9'))
        && version_bytes.all(|byte| byte.is_ascii_digit())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelProfileParameters(ProfileValue);

impl ModelProfileParameters {
    #[must_use]
    pub fn is_empty_mapping(&self) -> bool {
        matches!(&self.0, ProfileValue::Mapping(values) if values.is_empty())
    }

    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("validated profile values always serialize to JSON")
    }
}

impl Serialize for ModelProfileParameters {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModelProfileParameters {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ProfileValueVisitor).map(Self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProfileValue {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Float(u64),
    String(String),
    Sequence(Vec<Self>),
    Mapping(BTreeMap<String, Self>),
}

impl Serialize for ProfileValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Signed(value) => serializer.serialize_i64(*value),
            Self::Unsigned(value) => serializer.serialize_u64(*value),
            Self::Float(bits) => serializer.serialize_f64(f64::from_bits(*bits)),
            Self::String(value) => serializer.serialize_str(value),
            Self::Sequence(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            },
            Self::Mapping(values) => {
                let mut mapping = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    mapping.serialize_entry(key, value)?;
                }
                mapping.end()
            },
        }
    }
}

struct ProfileValueVisitor;

impl<'de> Visitor<'de> for ProfileValueVisitor {
    type Value = ProfileValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "a profile null, boolean, finite number, string, sequence, or string-keyed mapping",
        )
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(ProfileValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(ProfileValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(ProfileValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        if value < 0 {
            Ok(ProfileValue::Signed(value))
        } else {
            Ok(ProfileValue::Unsigned(value.cast_unsigned()))
        }
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(ProfileValue::Unsigned(value))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value < 0 {
            i64::try_from(value)
                .map(ProfileValue::Signed)
                .map_err(|_| E::custom("negative profile integer is outside i64"))
        } else {
            u64::try_from(value)
                .map(ProfileValue::Unsigned)
                .map_err(|_| E::custom("nonnegative profile integer is outside u64"))
        }
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        u64::try_from(value)
            .map(ProfileValue::Unsigned)
            .map_err(|_| E::custom("profile integer is outside u64"))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if !value.is_finite() {
            return Err(E::custom("profile floating-point value must be finite"));
        }
        let normalized = if value == 0.0 { 0.0 } else { value };
        Ok(ProfileValue::Float(normalized.to_bits()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(ProfileValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(ProfileValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element_seed(ProfileValueSeed)? {
            values.push(value);
        }
        Ok(ProfileValue::Sequence(values))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = mapping.next_key_seed(ProfileMapKeySeed)? {
            let value = mapping.next_value_seed(ProfileValueSeed)?;
            if values.insert(key.clone(), value).is_some() {
                return Err(A::Error::custom(format!(
                    "duplicate profile mapping key {key:?}"
                )));
            }
        }
        Ok(ProfileValue::Mapping(values))
    }
}

struct ProfileMapKeySeed;

impl<'de> serde::de::DeserializeSeed<'de> for ProfileMapKeySeed {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ProfileMapKeyVisitor)
    }
}

struct ProfileMapKeyVisitor;

impl<'de> Visitor<'de> for ProfileMapKeyVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string profile mapping key")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(value)
    }
}

struct ProfileValueSeed;

impl<'de> serde::de::DeserializeSeed<'de> for ProfileValueSeed {
    type Value = ProfileValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ProfileValueVisitor)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelProfileLayer {
    api_dialect: Option<ApiDialect>,
    tokenizer_profile: Option<VersionedProfileId>,
    input_token_limit: Option<u64>,
    max_output_tokens: Option<u64>,
    reasoning_parameters: Option<ModelProfileParameters>,
    optional_request_parameters: Option<ModelProfileParameters>,
    tool_capability_policy: Option<VersionedProfileId>,
    verification_profile: Option<VersionedProfileId>,
    replay_profile: Option<VersionedProfileId>,
}

pub const SEMANTIC_REPLAY_PROFILE: &str = "semantic-only/v1";
pub const KIMI_PRIVATE_REPLAY_PROFILE: &str = "kimi-private-local-plaintext/v1";

impl ModelProfileLayer {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        api_dialect: Option<ApiDialect>,
        tokenizer_profile: Option<VersionedProfileId>,
        input_token_limit: Option<u64>,
        max_output_tokens: Option<u64>,
        reasoning_parameters: Option<ModelProfileParameters>,
        optional_request_parameters: Option<ModelProfileParameters>,
        tool_capability_policy: Option<VersionedProfileId>,
        verification_profile: Option<VersionedProfileId>,
    ) -> Self {
        Self {
            api_dialect,
            tokenizer_profile,
            input_token_limit,
            max_output_tokens,
            reasoning_parameters,
            optional_request_parameters,
            tool_capability_policy,
            verification_profile,
            replay_profile: None,
        }
    }

    #[must_use]
    pub fn with_replay_profile(mut self, replay_profile: Option<VersionedProfileId>) -> Self {
        self.replay_profile = replay_profile;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveModelProfile {
    api_dialect: ApiDialect,
    context: ModelContextProfile,
    reasoning_parameters: ModelProfileParameters,
    optional_request_parameters: ModelProfileParameters,
    tool_capability_policy: VersionedProfileId,
    verification_profile: VersionedProfileId,
    replay_profile: VersionedProfileId,
}

impl EffectiveModelProfile {
    pub fn resolve(
        base: Option<&ModelProfileLayer>,
        model: &ModelProfileLayer,
    ) -> Result<Self, ModelServiceError> {
        let base = base.cloned().unwrap_or_default();
        let api_dialect = required("api_dialect", model.api_dialect.or(base.api_dialect))?;
        let tokenizer_profile = required(
            "tokenizer_profile",
            model.tokenizer_profile.clone().or(base.tokenizer_profile),
        )?;
        let input_token_limit = required(
            "input_token_limit",
            model.input_token_limit.or(base.input_token_limit),
        )?;
        let max_output_tokens = required(
            "max_output_tokens",
            model.max_output_tokens.or(base.max_output_tokens),
        )?;
        let reasoning_parameters = required(
            "reasoning_parameters",
            model
                .reasoning_parameters
                .clone()
                .or(base.reasoning_parameters),
        )?;
        let optional_request_parameters = required(
            "optional_request_parameters",
            model
                .optional_request_parameters
                .clone()
                .or(base.optional_request_parameters),
        )?;
        let tool_capability_policy = required(
            "tool_capability_policy",
            model
                .tool_capability_policy
                .clone()
                .or(base.tool_capability_policy),
        )?;
        let verification_profile = required(
            "verification_profile",
            model
                .verification_profile
                .clone()
                .or(base.verification_profile),
        )?;
        let replay_profile = model
            .replay_profile
            .clone()
            .or(base.replay_profile)
            .map_or_else(|| VersionedProfileId::new(SEMANTIC_REPLAY_PROFILE), Ok)?;
        let context = ModelContextProfile::from_versioned(
            input_token_limit,
            max_output_tokens,
            tokenizer_profile,
        )?;
        Ok(Self {
            api_dialect,
            context,
            reasoning_parameters,
            optional_request_parameters,
            tool_capability_policy,
            verification_profile,
            replay_profile,
        })
    }

    #[must_use]
    pub const fn api_dialect(&self) -> ApiDialect {
        self.api_dialect
    }

    #[must_use]
    pub const fn context(&self) -> &ModelContextProfile {
        &self.context
    }

    #[must_use]
    pub const fn reasoning_parameters(&self) -> &ModelProfileParameters {
        &self.reasoning_parameters
    }

    #[must_use]
    pub const fn optional_request_parameters(&self) -> &ModelProfileParameters {
        &self.optional_request_parameters
    }

    #[must_use]
    pub const fn tool_capability_policy(&self) -> &VersionedProfileId {
        &self.tool_capability_policy
    }

    #[must_use]
    pub const fn verification_profile(&self) -> &VersionedProfileId {
        &self.verification_profile
    }

    #[must_use]
    pub const fn replay_profile(&self) -> &VersionedProfileId {
        &self.replay_profile
    }
}

fn required<T>(name: &str, value: Option<T>) -> Result<T, ModelServiceError> {
    value.ok_or_else(|| ModelServiceError::new(format!("resolved model profile is missing {name}")))
}
