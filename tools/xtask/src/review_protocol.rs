//! Shared review-packet and review-delta protocol primitives.
//!
//! This module owns only byte-identical wire types and pure identity/path
//! helpers. Workflow-specific validation, rendering, diagnostics, token
//! budgeting, and storage remain with the packet and delta owners.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const TOKENIZER_PROFILE: &str = "o200k_base/v1";
pub(super) const TOKENIZER_COMPILER: &str = "tiktoken-rs/0.12.0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EvidenceRequest {
    pub(super) name: String,
    pub(super) path: String,
}

#[derive(Clone, Debug)]
pub(super) struct Captured {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct NamedCaptured {
    pub(super) name: String,
    pub(super) artifact: Captured,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Artifact {
    pub(super) path: String,
    pub(super) hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct NamedArtifact {
    pub(super) name: String,
    pub(super) artifact: Artifact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct NamedSemanticInput {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct DeliveryProfile {
    pub(super) id: String,
    pub(super) preamble: String,
    pub(super) section_prefix: String,
    pub(super) metadata_suffix: String,
    pub(super) section_suffix: String,
    pub(super) payload_suffix: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PacketRecord {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) managed_payload_tokens: usize,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct ArtifactWithTokens {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) managed_payload_tokens: usize,
}

pub(super) fn artifact(input: &Captured) -> Artifact {
    Artifact {
        path: input.path.clone(),
        hash: input.hash.clone(),
    }
}

pub(super) fn sorted_unique(values: &[String], label: &str) -> Result<Vec<String>, String> {
    let mut sorted = values.to_vec();
    if sorted.iter().any(|value| value.trim().is_empty()) {
        return Err(format!("{label} must not be blank"));
    }
    sorted.sort();
    let original = sorted.len();
    sorted.dedup();
    if sorted.len() != original {
        return Err(format!("{label} values must be unique"));
    }
    Ok(sorted)
}

pub(super) fn require_commit(commit: &str, label: &str) -> Result<(), String> {
    if commit.len() == 40
        && commit
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(format!("{label} must be a full lowercase SHA-1 commit ID"))
    }
}

pub(super) fn resolve_input_path(repository: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository.join(path)
    }
}

pub(super) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn digest(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).as_slice())
}

pub(super) fn domain_digest(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((domain.len() as u64).to_be_bytes());
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    hex_digest(hasher.finalize().as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
