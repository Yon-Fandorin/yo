use std::{
    fs::canonicalize,
    io::Error,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    task::{Context, Poll},
    thread,
};

use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags, open, openat},
    io::Errno,
};

use super::{inventory::build_inventory, ranking::search};
use crate::{
    WorkspaceHostId, WorkspaceReferenceProvider, WorkspaceReferenceProviderPoll,
    WorkspaceReferenceSearchRequest, WorkspaceReferenceSearchStatus,
    WorkspaceReferenceSearchUpdate,
    readiness::{Readiness, ReadyReceiver},
};

pub struct LocalWorkspaceReferenceProvider {
    requests: Sender<WorkspaceReferenceSearchRequest>,
    updates: ReadyReceiver<WorkspaceReferenceSearchUpdate>,
}

pub(super) fn pin_root(root: &Path) -> Result<OwnedFd, Errno> {
    let filesystem = open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    pin_directory(
        &filesystem,
        root.strip_prefix("/").map_err(|_| Errno::INVAL)?,
    )
}

pub(super) fn pin_directory(root: &OwnedFd, relative: &Path) -> Result<OwnedFd, Errno> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = openat(root, ".", flags, Mode::empty())?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(Errno::INVAL);
        };
        directory = openat(&directory, name, flags, Mode::empty())?;
    }
    Ok(directory)
}

impl LocalWorkspaceReferenceProvider {
    pub fn start(root: &Path, workspace_host_id: WorkspaceHostId) -> Result<Self, Error> {
        let (request_tx, request_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        let readiness = Arc::new(Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let root = canonicalize(root)?;
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
            updates: ReadyReceiver::new(update_rx, readiness),
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

pub(super) fn worker(
    root: PathBuf,
    workspace_host_id: WorkspaceHostId,
    requests: Receiver<WorkspaceReferenceSearchRequest>,
    updates: Sender<WorkspaceReferenceSearchUpdate>,
    readiness: &Readiness,
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
