//! Codex skill catalog worker의 lifecycle과 request coalescing을 담당합니다.

use std::{
    io::Error,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
};

use yo_backend::transport::{Readiness, ReadyReceiver};
use yo_core::{
    SkillReferenceCandidate, SkillReferenceProvider, SkillReferenceProviderPoll,
    SkillReferenceSearchRequest, SkillReferenceSearchStatus, SkillReferenceSearchUpdate,
    WorkspaceHostId, search_skill_reference_candidates,
};

use super::{
    catalog::{candidate_from_wire, format_catalog_errors, load_skill_metadata},
    resolution::skill_digest,
};
use crate::{CodexBackendConfig, CodexWarningObserver};

struct Inventory {
    candidates: Vec<SkillReferenceCandidate>,
    status: SkillReferenceSearchStatus,
}

/// Codex catalog 연결을 소유하는 비동기 skill reference provider입니다.
pub struct CodexSkillReferenceProvider {
    requests: Option<Sender<SkillReferenceSearchRequest>>,
    updates: ReadyReceiver<SkillReferenceSearchUpdate>,
    worker: Option<JoinHandle<()>>,
}

impl CodexSkillReferenceProvider {
    /// 짧은 수명의 Codex catalog 연결을 소유하는 worker를 시작합니다.
    pub fn start(
        config: CodexBackendConfig,
        workspace_host_id: WorkspaceHostId,
    ) -> Result<Self, Error> {
        Self::start_with_warning_observer(config, workspace_host_id, None)
    }

    /// compatibility observation을 호출자가 소유한 observer로 전달하며 worker를 시작합니다.
    pub fn start_with_warning_observer(
        config: CodexBackendConfig,
        workspace_host_id: WorkspaceHostId,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<Self, Error> {
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

pub(super) fn newest_request(
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
