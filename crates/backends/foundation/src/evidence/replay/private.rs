use std::{collections::HashSet, fmt};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as SerdeDeError, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};

use super::{super::valid_schema, budget::MAX_REPLAY_TEXT_BYTES};

#[derive(Clone, Eq, PartialEq)]
pub struct ProviderPrivateReplayEnvelope {
    schema: String,
    payload: Vec<u8>,
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderPrivateReplayPayload {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl fmt::Debug for ProviderPrivateReplayEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderPrivateReplayEnvelope")
            .field("schema", &self.schema)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

impl ProviderPrivateReplayEnvelope {
    pub fn new(schema: impl Into<String>, payload: Vec<u8>) -> Result<Self, &'static str> {
        let envelope = Self {
            schema: schema.into(),
            payload,
        };
        if !envelope.is_valid() {
            return Err("provider-private replay envelope is invalid or exceeds its bounds");
        }
        Ok(envelope)
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[doc(hidden)]
    pub fn ordered_payload(&self) -> ProviderPrivateReplayPayload {
        serde_json::from_slice(&self.payload)
            .expect("a validated provider-private payload remains ordered canonical JSON")
    }

    fn is_valid(&self) -> bool {
        valid_versioned_schema(&self.schema)
            && !self.payload.is_empty()
            && self.payload.len() <= MAX_REPLAY_TEXT_BYTES
            && serde_json::from_slice::<ProviderPrivateReplayPayload>(&self.payload).is_ok_and(
                |value| {
                    matches!(value, ProviderPrivateReplayPayload::Object(_))
                        && serde_json::to_vec(&value)
                            .is_ok_and(|canonical| canonical == self.payload)
                },
            )
    }
}

pub(super) fn is_valid_envelope(envelope: &ProviderPrivateReplayEnvelope) -> bool {
    envelope.is_valid()
}

impl Serialize for ProviderPrivateReplayPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            },
            Self::Object(fields) => {
                let mut mapping = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    mapping.serialize_entry(key, value)?;
                }
                mapping.end()
            },
        }
    }
}

impl<'de> Deserialize<'de> for ProviderPrivateReplayPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PayloadVisitor;

        impl<'de> Visitor<'de> for PayloadVisitor {
            type Value = ProviderPrivateReplayPayload;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object members")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Null)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: SerdeDeError,
            {
                serde_json::Number::from_f64(value)
                    .map(ProviderPrivateReplayPayload::Number)
                    .ok_or_else(|| E::custom("provider-private JSON number is not finite"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::String(value))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(ProviderPrivateReplayPayload::Array(values))
            }

            fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut keys = HashSet::new();
                let mut fields = Vec::new();
                while let Some((key, value)) = mapping.next_entry::<String, _>()? {
                    if !keys.insert(key.clone()) {
                        return Err(A::Error::custom("duplicate provider-private JSON member"));
                    }
                    fields.push((key, value));
                }
                Ok(ProviderPrivateReplayPayload::Object(fields))
            }
        }

        deserializer.deserialize_any(PayloadVisitor)
    }
}

fn valid_versioned_schema(value: &str) -> bool {
    let Some((name, version)) = value.rsplit_once("/v") else {
        return false;
    };
    valid_schema(value)
        && !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && version.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
