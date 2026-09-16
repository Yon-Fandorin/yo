use std::{env, process::Command};

use super::{
    command::canonical_sha256,
    model::{ResourceLease, ReuseContext},
    parse::{schema, unsupported_schema},
    schemas,
};
use crate::{review_packet::external_operation, review_protocol};

pub(super) const REVIEWED_DESCENDANT_REUSE: &str = "reviewed-descendant/v1";
pub(super) const CONTEXT_REUSE: &str = "reviewed-descendant-context/v1";
const REUSE_CONTEXT_SCHEMA: &str = "yo.validation-reuse-context/v1alpha1";
const NO_EXTERNAL_STATE: &str = "none-declared";
const TOOLCHAIN_DOMAIN: &[u8] = b"yo.validation-toolchain/v1alpha1\0";

/// Returns whether this exact validation result carries a reusable local
/// execution context that still matches the current host.  Callers use this
/// only to replace an otherwise duplicate local validation; a legacy or
/// external-state summary remains valid gate evidence but is not eligible for
/// that fast path.
pub(crate) fn current_reusable_context(bytes: &[u8]) -> Result<bool, String> {
    let envelope = schema(bytes)?;
    match envelope.schema.as_str() {
        schemas::ALPHA3_SCHEMA => {
            let context = schemas::alpha3_reuse_context(bytes)?;
            verify_reuse_context_format(&context)?;
            reusable_context_matches_current(&context)
        },
        schemas::ALPHA4_SCHEMA => {
            let Some(context) = schemas::alpha4_reuse_context(bytes)? else {
                return Ok(false);
            };
            verify_reuse_context_format(&context)?;
            reusable_context_matches_current(&context)
        },
        schemas::LEGACY_SCHEMA
        | schemas::ALPHA1_SCHEMA
        | schemas::ALPHA2_SCHEMA
        | external_operation::SCHEMA => Ok(false),
        other => unsupported_schema(other),
    }
}

pub(super) fn verify_resource_lease(lease: &ResourceLease) -> Result<(), String> {
    if lease.schema != "yo.validation-resource-lease/v1alpha1"
        || !matches!(lease.class.as_str(), "cargo-heavy" | "independent")
        || lease.key.is_empty()
        || lease.key.len() > 128
        || lease
            .key
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        || lease.status != "acquired"
        || lease.wait_attempts != 0
    {
        return Err(
            "validation resource lease is not a closed acquired v1alpha1 observation".to_owned(),
        );
    }
    Ok(())
}

pub(super) fn verify_reuse_context_format(context: &ReuseContext) -> Result<(), String> {
    if context.schema != REUSE_CONTEXT_SCHEMA {
        return Err(format!(
            "reuse_context schema must be `{REUSE_CONTEXT_SCHEMA}`"
        ));
    }
    if context.platform_os.is_empty() || context.platform_arch.is_empty() {
        return Err("reuse_context platform fields must not be empty".to_owned());
    }
    canonical_sha256(&context.toolchain_hash, "validation toolchain hash")?;
    if context.external_state != NO_EXTERNAL_STATE {
        return Err(format!(
            "reuse_context external_state must be `{NO_EXTERNAL_STATE}`"
        ));
    }
    Ok(())
}

pub(super) fn verify_current_reuse_context(context: &ReuseContext) -> Result<(), String> {
    if context.platform_os != env::consts::OS || context.platform_arch != env::consts::ARCH {
        return Err(format!(
            "validation reuse context platform changed from {}/{} to {}/{}",
            context.platform_os,
            context.platform_arch,
            env::consts::OS,
            env::consts::ARCH
        ));
    }
    let current_toolchain = current_toolchain_hash()?;
    if context.toolchain_hash != current_toolchain {
        return Err(format!(
            "validation reuse context toolchain changed; expected {} but found {current_toolchain}",
            context.toolchain_hash
        ));
    }
    Ok(())
}

fn reusable_context_matches_current(context: &ReuseContext) -> Result<bool, String> {
    Ok(context.platform_os == env::consts::OS
        && context.platform_arch == env::consts::ARCH
        && context.toolchain_hash == current_toolchain_hash()?)
}

pub(super) fn current_toolchain_hash() -> Result<String, String> {
    let mut framed = Vec::from(TOOLCHAIN_DOMAIN);
    for tool in ["rustc", "cargo"] {
        let output = Command::new(tool)
            .arg("-Vv")
            .output()
            .map_err(|error| format!("cannot fingerprint {tool} -Vv: {error}"))?;
        if !output.status.success() {
            return Err(format!("cannot fingerprint {tool} -Vv"));
        }
        let value = String::from_utf8(output.stdout)
            .map_err(|_| format!("{tool} -Vv returned non-UTF-8 output"))?;
        let value = value.trim_end_matches('\n').as_bytes();
        framed.extend_from_slice(value.len().to_string().as_bytes());
        framed.push(b':');
        framed.extend_from_slice(value);
        framed.push(0);
    }
    Ok(review_protocol::digest(&framed))
}
