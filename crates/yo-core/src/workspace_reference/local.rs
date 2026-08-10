//! Local execution-workspace discovery kept outside the terminal UI thread.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    task::{Context, Poll},
    thread,
};

#[cfg(test)]
use self::{
    filesystem::discover_entries,
    git::{classify_git_workspace, git_command, is_git_workspace},
    ranking::rank,
};
use self::{inventory::build_inventory, ranking::search};
use super::{
    WorkspaceReferenceProvider, WorkspaceReferenceProviderPoll, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchStatus, WorkspaceReferenceSearchUpdate,
};
use crate::WorkspaceHostId;

mod filesystem;
mod git;
mod inventory;
mod ranking;

pub struct LocalWorkspaceReferenceProvider {
    requests: Sender<WorkspaceReferenceSearchRequest>,
    updates: crate::readiness::ReadyReceiver<WorkspaceReferenceSearchUpdate>,
}

impl LocalWorkspaceReferenceProvider {
    pub fn start(root: &Path, workspace_host_id: WorkspaceHostId) -> Result<Self, std::io::Error> {
        let (request_tx, request_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        let readiness = Arc::new(crate::readiness::Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let root = std::fs::canonicalize(root)?;
        thread::Builder::new()
            .name("yo-workspace-search".to_owned())
            .spawn(move || {
                worker(
                    root,
                    workspace_host_id,
                    request_rx,
                    update_tx,
                    &worker_readiness,
                );
                worker_readiness.notify();
            })?;
        Ok(Self {
            requests: request_tx,
            updates: crate::readiness::ReadyReceiver::new(update_rx, readiness),
        })
    }
}

impl WorkspaceReferenceProvider for LocalWorkspaceReferenceProvider {
    fn search(&mut self, request: WorkspaceReferenceSearchRequest) -> Result<(), String> {
        self.requests
            .send(request)
            .map_err(|_| "workspace search worker closed".to_owned())
    }

    fn poll(&mut self) -> Result<WorkspaceReferenceProviderPoll, String> {
        match self.updates.try_recv() {
            Ok(update) => Ok(WorkspaceReferenceProviderPoll::Update(update)),
            Err(TryRecvError::Empty) => Ok(WorkspaceReferenceProviderPoll::Pending),
            Err(TryRecvError::Disconnected) => Err("workspace search worker closed".to_owned()),
        }
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.updates.poll_ready(context)
    }
}

fn worker(
    root: PathBuf,
    workspace_host_id: WorkspaceHostId,
    requests: Receiver<WorkspaceReferenceSearchRequest>,
    updates: Sender<WorkspaceReferenceSearchUpdate>,
    readiness: &crate::readiness::Readiness,
) {
    let inventory = build_inventory(&root, workspace_host_id);
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let update = match &inventory {
            Ok(inventory) => WorkspaceReferenceSearchUpdate::final_result(
                &request,
                inventory.status.clone(),
                search(&inventory.entries, request.query()),
            ),
            Err(error) => WorkspaceReferenceSearchUpdate::final_result(
                &request,
                WorkspaceReferenceSearchStatus::Failed(error.clone()),
                Vec::new(),
            ),
        };
        if updates.send(update).is_err() {
            break;
        }
        readiness.notify();
    }
}

#[cfg(test)]
mod tests;
