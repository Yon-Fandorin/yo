//! Codex `skills/list` wire 타입과 후보 projection을 담당합니다.

use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::json;
use yo_core::{SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceScope};

use crate::{
    CodexBackendConfig, CodexWarningObserver, client::AppServerClient, transport::StdioPeer,
};

#[derive(Deserialize)]
pub(super) struct SkillsListResponse {
    pub(super) data: Vec<SkillsListEntry>,
}

#[derive(Deserialize)]
pub(super) struct SkillsListEntry {
    pub(super) cwd: String,
    pub(super) skills: Vec<SkillMetadata>,
    pub(super) errors: Vec<SkillErrorInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillMetadata {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) short_description: Option<String>,
    pub(super) path: String,
    pub(super) scope: WireScope,
    pub(super) enabled: bool,
    pub(super) interface: Option<SkillInterface>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInterface {
    pub(super) display_name: Option<String>,
    pub(super) short_description: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SkillErrorInfo {
    pub(super) message: String,
    pub(super) path: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum WireScope {
    User,
    Repo,
    System,
    Admin,
}

pub(super) fn load_skill_metadata(
    config: &CodexBackendConfig,
    warning_observer: Option<CodexWarningObserver>,
) -> Result<SkillsListEntry, String> {
    let cwd = config
        .working_directory()
        .to_str()
        .ok_or_else(|| "Codex skill catalog working directory is not valid UTF-8".to_owned())?;
    let peer = StdioPeer::spawn(config).map_err(|error| error.to_string())?;
    let mut client = AppServerClient::new(peer, config.request_timeout())
        .with_warning_observer(warning_observer);
    let result = (|| {
        client.initialize().map_err(|error| error.to_string())?;
        let value = client
            .call("skills/list", json!({ "cwds": [cwd], "forceReload": true }))
            .map_err(|error| error.to_string())?
            .result;
        serde_json::from_value::<SkillsListResponse>(value)
            .map_err(|error| format!("invalid Codex skills/list response: {error}"))
    })();
    let _ = client.shutdown();
    let response = result?;
    let entry = response
        .data
        .into_iter()
        .find(|entry| entry.cwd == cwd)
        .ok_or_else(|| "Codex skills/list omitted the requested workspace".to_owned())?;
    Ok(entry)
}

pub(super) fn candidate_from_wire(
    environment: &str,
    skill: SkillMetadata,
    catalog_generation: u64,
    revision: Result<String, String>,
) -> SkillReferenceCandidate {
    let scope = match skill.scope {
        WireScope::Repo => SkillReferenceScope::Workspace,
        WireScope::User => SkillReferenceScope::User,
        WireScope::System => SkillReferenceScope::System,
        WireScope::Admin => SkillReferenceScope::Admin,
    };
    let display_name = skill
        .interface
        .as_ref()
        .and_then(|interface| interface.display_name.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| skill.name.clone());
    let description = skill
        .interface
        .as_ref()
        .and_then(|interface| interface.short_description.clone())
        .or(skill.short_description.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| skill.description.clone());
    let identity = semantic_key("codex-skill", &[environment, &skill.path]);
    let entry_revision = revision.as_deref().unwrap_or("unavailable");
    let reference = SkillReference::new(
        identity,
        environment,
        &skill.path,
        skill.name,
        scope,
        catalog_generation,
        entry_revision,
    );
    let availability = if !skill.enabled {
        SkillAvailability::Disabled("Disabled by Codex configuration".to_owned())
    } else if let Err(reason) = revision {
        SkillAvailability::Disabled(reason)
    } else {
        SkillAvailability::Enabled
    };
    SkillReferenceCandidate::new(reference, display_name, description, availability)
}

pub(super) fn format_catalog_errors(errors: &[SkillErrorInfo]) -> String {
    let first = &errors[0];
    if errors.len() == 1 {
        format!("Codex skipped {}: {}", first.path, first.message)
    } else {
        format!(
            "Codex skipped {} skill entries; first at {}: {}",
            errors.len(),
            first.path,
            first.message
        )
    }
}

fn semantic_key(domain: &str, fields: &[&str]) -> String {
    let mut value = String::from(domain);
    for field in fields {
        value.push(':');
        value.push_str(&field.len().to_string());
        value.push(':');
        for byte in field.as_bytes() {
            let _ = write!(value, "{byte:02x}");
        }
    }
    value
}
