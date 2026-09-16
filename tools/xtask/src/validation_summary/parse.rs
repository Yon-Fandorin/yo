use serde::Deserialize;

use super::{
    model::SchemaEnvelope,
    schemas::{ALPHA1_SCHEMA, ALPHA2_SCHEMA, ALPHA3_SCHEMA, ALPHA4_SCHEMA, LEGACY_SCHEMA},
};

pub(super) fn schema(bytes: &[u8]) -> Result<SchemaEnvelope, String> {
    serde_json::from_slice(bytes).map_err(|error| format!("cannot read summary schema: {error}"))
}

pub(super) fn parse<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

pub(super) fn unsupported_schema<T>(schema: &str) -> Result<T, String> {
    Err(format!(
        "unsupported schema `{schema}`; expected `{LEGACY_SCHEMA}`, `{ALPHA1_SCHEMA}`, `{ALPHA2_SCHEMA}`, `{ALPHA3_SCHEMA}`, `{ALPHA4_SCHEMA}`, or `{}`",
        crate::review_packet::external_operation::SCHEMA,
    ))
}
