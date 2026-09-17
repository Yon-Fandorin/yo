use std::path::Path;

use yo_core::{AccountId, ModelId, ProviderId};

use super::model::{
    REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2, REQUEST_SCHEMA_V1_ALPHA3, REQUEST_SCHEMA_V1_ALPHA4,
    REQUEST_SCHEMA_V1_ALPHA5, REQUEST_SCHEMA_V1_ALPHA6, Request, ReviewTarget,
};
use crate::bounded_file;

const REQUEST_LIMIT: usize = 64 * 1024;
const MAX_TOKEN_BYTES: usize = 128;

pub(super) fn read(path: &Path) -> Result<Request, String> {
    let bytes = bounded_file::read_regular(
        path,
        REQUEST_LIMIT,
        "external review target admission request",
    )?;
    let request: Request = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid external review target admission request {}: {error}",
            path.display()
        )
    })?;
    validate(&request)?;
    Ok(request)
}

pub(super) fn validate(request: &Request) -> Result<(), String> {
    if !matches!(
        request.schema.as_str(),
        REQUEST_SCHEMA
            | REQUEST_SCHEMA_V1_ALPHA2
            | REQUEST_SCHEMA_V1_ALPHA3
            | REQUEST_SCHEMA_V1_ALPHA4
            | REQUEST_SCHEMA_V1_ALPHA5
            | REQUEST_SCHEMA_V1_ALPHA6
    ) {
        return Err(format!(
            "unsupported external review target admission request schema `{}`; expected `{REQUEST_SCHEMA}`, `{REQUEST_SCHEMA_V1_ALPHA2}`, `{REQUEST_SCHEMA_V1_ALPHA3}`, `{REQUEST_SCHEMA_V1_ALPHA4}`, `{REQUEST_SCHEMA_V1_ALPHA5}`, or `{REQUEST_SCHEMA_V1_ALPHA6}`",
            request.schema
        ));
    }
    validate_target(&request.target)?;
    match (&request.target, &request.connection_repository_path) {
        (ReviewTarget::ManagedModel { .. }, Some(path)) => {
            compact_absolute_path(path, "connection_repository_path")?
        },
        (ReviewTarget::ManagedModel { .. }, None) => {
            return Err("a managed-model admission requires connection_repository_path".to_owned());
        },
        (ReviewTarget::DelegatedHost { .. }, Some(_)) => {
            return Err(
                "a delegated-host admission must not name connection_repository_path".to_owned(),
            );
        },
        (ReviewTarget::DelegatedHost { .. }, None) => {},
    }
    if let Some(path) = &request.session_repository_path {
        compact_absolute_path(path, "session_repository_path")?;
    }
    Ok(())
}

pub(super) fn validate_target(target: &ReviewTarget) -> Result<(), String> {
    match target {
        ReviewTarget::ManagedModel {
            provider,
            account,
            model,
        } => {
            compact_token(provider, "target provider")?;
            compact_token(account, "target account")?;
            compact_token(model, "target model")?;
            if [provider, account, model]
                .into_iter()
                .any(|value| value.contains(':'))
            {
                return Err("managed review-target coordinates must not contain `:`".to_owned());
            }
            ProviderId::new(provider.clone()).map_err(|error| error.to_string())?;
            AccountId::new(account.clone()).map_err(|error| error.to_string())?;
            ModelId::new(model.clone()).map_err(|error| error.to_string())?;
        },
        ReviewTarget::DelegatedHost { host } => {
            compact_token(host, "target host")?;
            if !matches!(host.as_str(), "codex" | "grok") {
                return Err(
                    "delegated review target must be exact host `codex` or `grok`".to_owned(),
                );
            }
        },
    }
    Ok(())
}

fn compact_path(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!(
            "{label} must be a non-empty path of at most 4096 bytes"
        ))
    } else {
        Ok(())
    }
}

fn compact_absolute_path(value: &str, label: &str) -> Result<(), String> {
    compact_path(value, label)?;
    if Path::new(value).is_absolute() {
        Ok(())
    } else {
        Err(format!("{label} must be absolute"))
    }
}

fn compact_token(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_TOKEN_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_whitespace())
    {
        Err(format!(
            "{label} must be a non-empty visible ASCII token of at most {MAX_TOKEN_BYTES} bytes"
        ))
    } else {
        Ok(())
    }
}
