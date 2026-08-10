//! Versioned revision-authoring facade and wire contracts.
//!
//! Exact request versions own their behavior below `v1alpha1/` and
//! `v1alpha2/`. Shared Source and Knowledge derivation lives in `shared`; a
//! version module must opt into every additional artifact it publishes.

use std::{fs, io::Read, path::Path};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::review::OperationFailure;

const OPERATION: &str = "author-revision";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
mod records;
mod shared;
mod v1alpha1;
mod v1alpha2;

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum AuthorSuccess {
    V1Alpha1(v1alpha1::Success),
    V1Alpha2(v1alpha2::Success),
}

pub(crate) struct AuthorService<'a> {
    repository_root: &'a Path,
}

impl<'a> AuthorService<'a> {
    pub(crate) fn new(repository_root: &'a Path) -> Self {
        Self { repository_root }
    }

    pub(crate) fn author_revision(
        &self,
        request_path: &Path,
    ) -> Result<AuthorSuccess, OperationFailure> {
        let bytes = read_request_bytes(request_path)?;
        let request: Value = decode_request(&bytes)?;
        match request.get("schema").and_then(Value::as_str) {
            Some(v1alpha2::REQUEST_SCHEMA) => {
                v1alpha2::author_revision(self.repository_root, &bytes)
            },
            // Every input not explicitly selecting v1alpha2 follows the
            // original v1alpha1 typed decoder. This preserves its missing,
            // mistyped, unknown-field, and unsupported-schema diagnostics.
            _ => v1alpha1::author_revision(self.repository_root, &bytes),
        }
    }
}

fn read_request_bytes(path: &Path) -> Result<Vec<u8>, OperationFailure> {
    let mut file = fs::File::open(path).map_err(|error| {
        OperationFailure::new(
            OPERATION,
            "request_unreadable",
            error.to_string(),
            Vec::new(),
            "provide a readable versioned JSON request file",
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            OperationFailure::new(
                OPERATION,
                "request_unreadable",
                error.to_string(),
                Vec::new(),
                "provide a readable versioned JSON request file",
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(OperationFailure::new(
            OPERATION,
            "request_too_large",
            format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            Vec::new(),
            "reduce the request to the required fields",
        ));
    }
    Ok(bytes)
}

fn decode_request<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, OperationFailure> {
    serde_json::from_slice(bytes).map_err(|error| {
        OperationFailure::new(
            OPERATION,
            "invalid_request",
            error.to_string(),
            Vec::new(),
            "repair the versioned JSON request",
        )
    })
}
