//! Codex `skills/list` catalog adapter kept off the terminal event loop.

use std::{
    fmt::Write as _,
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::OpenOptionsExt as _,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
};

use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use yo_backend::transport::{Readiness, ReadyReceiver};
use yo_core::{
    InputAdmissionHost, InputReference, LocalWorkspaceInputAdmission, ResolvedSkill,
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceProvider,
    SkillReferenceProviderPoll, SkillReferenceScope, SkillReferenceSearchRequest,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate, SubmissionRejection,
    SubmissionRejectionKind, UserInput, WorkspaceHostId, search_skill_reference_candidates,
};

use crate::{
    CodexBackendConfig, CodexWarningObserver, client::AppServerClient, transport::StdioPeer,
};

pub struct CodexSkillReferenceProvider {
    requests: Option<Sender<SkillReferenceSearchRequest>>,
    updates: ReadyReceiver<SkillReferenceSearchUpdate>,
    worker: Option<JoinHandle<()>>,
}

/// Execution-host admission using Codex's authoritative explicit-skill catalog.
/// The runtime consumes this through the provider-neutral InputAdmissionHost port.
pub struct CodexSkillInputAdmission {
    config: CodexBackendConfig,
    workspace_host_id: WorkspaceHostId,
    workspace: LocalWorkspaceInputAdmission,
    warning_observer: Option<CodexWarningObserver>,
}

impl CodexSkillInputAdmission {
    /// Pins workspace admission and the same host configuration used for discovery.
    pub fn new(
        config: CodexBackendConfig,
        workspace_host_id: WorkspaceHostId,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<Self, String> {
        let workspace =
            LocalWorkspaceInputAdmission::new(config.working_directory(), workspace_host_id)?;
        Ok(Self {
            config,
            workspace_host_id,
            workspace,
            warning_observer,
        })
    }

    fn validate_workspace(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        if input.references().len() > 128 {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::OverBudget,
                "at most 128 input references are supported",
            ));
        }
        let workspace_input = UserInput::with_references(
            input.as_str(),
            input
                .references()
                .iter()
                .filter(|reference| reference.workspace_reference().is_some())
                .cloned()
                .collect(),
        )
        .map_err(|error| {
            SubmissionRejection::new(SubmissionRejectionKind::InvalidReference, error.to_string())
        })?;
        self.workspace.validate(&workspace_input)
    }

    fn selected(&self, input: &UserInput) -> Result<Option<SkillReference>, SubmissionRejection> {
        self.validate_workspace(input)?;
        let Some(selected) = input
            .references()
            .iter()
            .find_map(InputReference::skill_reference)
        else {
            return Ok(None);
        };
        let environment = format!("local-host:{}", self.workspace_host_id);
        if selected.execution_environment_identity() != environment {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::EnvironmentUnavailable,
                "skill belongs to another execution host",
            ));
        }
        let catalog =
            load_skill_metadata(&self.config, self.warning_observer.clone()).map_err(|error| {
                SubmissionRejection::new(SubmissionRejectionKind::EnvironmentUnavailable, error)
            })?;
        validate_selected_skill(&environment, selected, catalog.skills)?;
        Ok(Some(selected.clone()))
    }
}

impl InputAdmissionHost for CodexSkillInputAdmission {
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        self.selected(input).map(|_| ())
    }

    fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        let Some(selected) = self.selected(input)? else {
            return Ok(None);
        };
        resolve_selected_skill(selected).map(Some)
    }
}

fn validate_selected_skill(
    environment: &str,
    selected: &SkillReference,
    skills: Vec<SkillMetadata>,
) -> Result<(), SubmissionRejection> {
    let mut matching = skills
        .into_iter()
        .filter(|skill| skill.path == selected.locator());
    let Some(skill) = matching.next() else {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::StaleReference,
            "selected skill was removed; select it again",
        ));
    };
    if matching.next().is_some() {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::InvalidReference,
            "skill catalog contains an ambiguous locator",
        ));
    }
    // Codex explicit selection uses enabled; allow_implicit_invocation is unrelated.
    let candidate = candidate_from_wire(
        environment,
        skill,
        selected.catalog_generation(),
        Ok(selected.entry_revision().to_owned()),
    );
    if candidate.reference() != selected {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::StaleReference,
            "selected skill descriptor changed; select it again",
        ));
    }
    if let SkillAvailability::Disabled(reason) = candidate.availability() {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::Unauthorized,
            reason,
        ));
    }
    Ok(())
}

fn resolve_selected_skill(selected: SkillReference) -> Result<ResolvedSkill, SubmissionRejection> {
    // The authoritative catalog was checked before opening its locator. Digest and model
    // instructions use these same bounded bytes, never a second path lookup.
    let instructions = skill_instructions(selected.locator())?;
    if skill_revision(&instructions) != selected.entry_revision() {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::StaleReference,
            "selected skill contents changed; select it again",
        ));
    }
    ResolvedSkill::new(selected, instructions).map_err(|error| {
        SubmissionRejection::new(
            SubmissionRejectionKind::RequiredAssetUnavailable,
            error.to_string(),
        )
    })
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
        Self::start_with_warning_observer(config, workspace_host_id, None)
    }

    /// Starts a worker and forwards compatibility observations to the caller-owned observer.
    pub fn start_with_warning_observer(
        config: CodexBackendConfig,
        workspace_host_id: WorkspaceHostId,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<Self, std::io::Error> {
        let (request_tx, request_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        let readiness = Arc::new(Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let worker = thread::Builder::new()
            .name("yo-codex-skill-catalog".to_owned())
            .spawn(move || {
                worker(
                    config,
                    workspace_host_id,
                    request_rx,
                    update_tx,
                    &worker_readiness,
                    warning_observer,
                );
                worker_readiness.notify();
            })?;
        Ok(Self {
            requests: Some(request_tx),
            updates: ReadyReceiver::new(update_rx, readiness),
            worker: Some(worker),
        })
    }
}

impl Drop for CodexSkillReferenceProvider {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl SkillReferenceProvider for CodexSkillReferenceProvider {
    fn search(&mut self, request: SkillReferenceSearchRequest) -> Result<(), String> {
        self.requests
            .as_ref()
            .ok_or_else(|| "Codex skill catalog worker closed".to_owned())?
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

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.updates.poll_ready(context)
    }
}

fn worker(
    config: CodexBackendConfig,
    workspace_host_id: WorkspaceHostId,
    requests: Receiver<SkillReferenceSearchRequest>,
    updates: Sender<SkillReferenceSearchUpdate>,
    readiness: &Readiness,
    warning_observer: Option<CodexWarningObserver>,
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
                warning_observer.clone(),
            ));
        }
        let update = match inventory
            .as_ref()
            .expect("the first request always attempts catalog loading")
        {
            Ok(inventory) => SkillReferenceSearchUpdate::final_result(
                &request,
                inventory.status.clone(),
                search_skill_reference_candidates(&inventory.candidates, request.query()),
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
        readiness.notify();
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
    warning_observer: Option<CodexWarningObserver>,
) -> Result<Inventory, String> {
    let entry = load_skill_metadata(config, warning_observer)?;
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

fn load_skill_metadata(
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
    skill_instructions(path)
        .map(|text| skill_revision(&text))
        .map_err(|error| error.message().to_owned())
}

fn skill_instructions(path: &str) -> Result<String, SubmissionRejection> {
    let unavailable = |message: String| {
        SubmissionRejection::new(SubmissionRejectionKind::RequiredAssetUnavailable, message)
    };
    // Open nonblocking before checking the same descriptor, so a replaced path
    // cannot leave the catalog worker waiting for a FIFO writer.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| unavailable(format!("Skill revision unavailable: {error}")))?;
    let metadata = file
        .metadata()
        .map_err(|error| unavailable(format!("Skill revision unavailable: {error}")))?;
    if !metadata.is_file() {
        return Err(unavailable(
            "Skill revision unavailable: not a regular file".to_owned(),
        ));
    }
    let limit = ResolvedSkill::MAX_INSTRUCTION_BYTES;
    if metadata.len() > limit as u64 {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::OverBudget,
            "Skill revision unavailable: instruction byte limit exceeded",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unavailable(format!("Skill revision unavailable: {error}")))?;
    if bytes.len() > limit {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::OverBudget,
            "Skill revision unavailable: instruction byte limit exceeded",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        unavailable("Skill revision unavailable: instructions are not valid UTF-8".to_owned())
    })
}

fn skill_revision(instructions: &str) -> String {
    let mut revision = String::from("sha256:");
    for byte in Sha256::digest(instructions.as_bytes()) {
        let _ = write!(revision, "{byte:02x}");
    }
    revision
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
