//! Codex `skills/list` catalog adapter kept off the terminal event loop.

use std::{
    fmt::Write as _,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};

use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{AppServerClient, CodexBackendConfig, StdioPeer};
use crate::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceProvider,
    SkillReferenceProviderPoll, SkillReferenceScope, SkillReferenceSearchRequest,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate, WorkspaceHostId,
    skill_reference::search_candidates,
};

pub struct CodexSkillReferenceProvider {
    requests: Sender<SkillReferenceSearchRequest>,
    updates: Receiver<SkillReferenceSearchUpdate>,
}

struct Inventory {
    candidates: Vec<SkillReferenceCandidate>,
    status: SkillReferenceSearchStatus,
}

#[derive(Deserialize)]
struct SkillsListResponse {
    data: Vec<SkillsListEntry>,
}

#[derive(Deserialize)]
struct SkillsListEntry {
    cwd: String,
    skills: Vec<SkillMetadata>,
    errors: Vec<SkillErrorInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillMetadata {
    name: String,
    description: String,
    short_description: Option<String>,
    path: String,
    scope: WireScope,
    enabled: bool,
    interface: Option<SkillInterface>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillInterface {
    display_name: Option<String>,
    short_description: Option<String>,
}

#[derive(Deserialize)]
struct SkillErrorInfo {
    message: String,
    path: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireScope {
    User,
    Repo,
    System,
    Admin,
}

impl CodexSkillReferenceProvider {
    /// Starts a worker that owns its own short-lived Codex catalog connection.
    pub fn start(
        config: CodexBackendConfig,
        workspace_host_id: WorkspaceHostId,
    ) -> Result<Self, std::io::Error> {
        let (request_tx, request_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        thread::Builder::new()
            .name("yo-codex-skill-catalog".to_owned())
            .spawn(move || worker(config, workspace_host_id, request_rx, update_tx))?;
        Ok(Self {
            requests: request_tx,
            updates: update_rx,
        })
    }
}

impl SkillReferenceProvider for CodexSkillReferenceProvider {
    fn search(&mut self, request: SkillReferenceSearchRequest) -> Result<(), String> {
        self.requests
            .send(request)
            .map_err(|_| "Codex skill catalog worker closed".to_owned())
    }

    fn poll(&mut self) -> Result<SkillReferenceProviderPoll, String> {
        match self.updates.try_recv() {
            Ok(update) => Ok(SkillReferenceProviderPoll::Update(update)),
            Err(TryRecvError::Empty) => Ok(SkillReferenceProviderPoll::Pending),
            Err(TryRecvError::Disconnected) => Err("Codex skill catalog worker closed".to_owned()),
        }
    }
}

fn worker(
    config: CodexBackendConfig,
    workspace_host_id: WorkspaceHostId,
    requests: Receiver<SkillReferenceSearchRequest>,
    updates: Sender<SkillReferenceSearchUpdate>,
) {
    let mut inventory = None;
    let mut catalog_generation = 0_u64;
    while let Ok(request) = requests.recv() {
        let (request, refresh_catalog) = newest_request(request, &requests);
        if refresh_catalog || inventory.is_none() {
            catalog_generation = catalog_generation.saturating_add(1);
            inventory = Some(load_inventory(
                &config,
                workspace_host_id,
                catalog_generation,
            ));
        }
        let update = match inventory
            .as_ref()
            .expect("the first request always attempts catalog loading")
        {
            Ok(inventory) => SkillReferenceSearchUpdate::final_result(
                &request,
                inventory.status.clone(),
                search_candidates(&inventory.candidates, request.query()),
            ),
            Err(reason) => SkillReferenceSearchUpdate::final_result(
                &request,
                SkillReferenceSearchStatus::Failed(reason.clone()),
                Vec::new(),
            ),
        };
        if updates.send(update).is_err() {
            return;
        }
    }
}

fn newest_request(
    mut request: SkillReferenceSearchRequest,
    requests: &Receiver<SkillReferenceSearchRequest>,
) -> (SkillReferenceSearchRequest, bool) {
    let mut refresh_catalog = request.refresh_catalog();
    while let Ok(newer) = requests.try_recv() {
        refresh_catalog |= newer.refresh_catalog();
        request = newer;
    }
    (request, refresh_catalog)
}

fn load_inventory(
    config: &CodexBackendConfig,
    workspace_host_id: WorkspaceHostId,
    catalog_generation: u64,
) -> Result<Inventory, String> {
    let cwd = config
        .working_directory()
        .to_str()
        .ok_or_else(|| "Codex skill catalog working directory is not valid UTF-8".to_owned())?;
    let peer = StdioPeer::spawn(config).map_err(|error| error.to_string())?;
    let mut client = AppServerClient::new(peer, config.request_timeout());
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
    let status = if entry.errors.is_empty() {
        SkillReferenceSearchStatus::Complete
    } else {
        SkillReferenceSearchStatus::Incomplete(format_catalog_errors(&entry.errors))
    };
    let environment = format!("local-host:{workspace_host_id}");
    let candidates = entry
        .skills
        .into_iter()
        .map(|skill| {
            let revision = skill_digest(&skill.path);
            candidate_from_wire(&environment, skill, catalog_generation, revision)
        })
        .collect();
    Ok(Inventory { candidates, status })
}

fn candidate_from_wire(
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

fn skill_digest(path: &str) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("Skill revision unavailable: {error}"))?;
    let mut revision = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        let _ = write!(revision, "{byte:02x}");
    }
    Ok(revision)
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

fn format_catalog_errors(errors: &[SkillErrorInfo]) -> String {
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

#[cfg(test)]
mod tests;
