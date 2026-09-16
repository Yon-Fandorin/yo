use std::{
    mem,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
};

use super::{
    super::{
        SkillReferenceProvider, SkillReferenceProviderPoll, SkillReferenceSearchRequest,
        SkillReferenceSearchStatus, SkillReferenceSearchUpdate, search_skill_reference_candidates,
    },
    catalog::Catalog,
    roots::LocalSkillRoot,
};
use crate::{WorkspaceHostId, readiness::Readiness};

#[derive(Default)]
struct Mailbox {
    request: Option<SkillReferenceSearchRequest>,
    refresh: bool,
    update: Option<SkillReferenceSearchUpdate>,
}

/// Nonblocking catalog connection with coalesced requests and a cancellable owned worker.
pub struct LocalSkillReferenceProvider {
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
    stopped: Arc<AtomicBool>,
    readiness: Arc<Readiness>,
    worker: Option<JoinHandle<()>>,
}

impl LocalSkillReferenceProvider {
    /// Starts bounded discovery for explicitly configured roots.
    pub fn start(roots: Vec<LocalSkillRoot>, host: WorkspaceHostId) -> Result<Self, String> {
        let catalog = Catalog::new(roots, host)?;
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let readiness = Arc::new(Readiness::new());
        let worker_shared = Arc::clone(&shared);
        let worker_stopped = Arc::clone(&stopped);
        let worker_readiness = Arc::clone(&readiness);
        let worker = thread::Builder::new()
            .name("yo-local-skill-catalog".into())
            .spawn(move || {
                let (lock, changed) = &*worker_shared;
                let mut cache = None;
                let mut generation = 0_u64;
                loop {
                    let (request, refresh) = {
                        let mut mailbox = lock.lock().unwrap_or_else(|error| error.into_inner());
                        while mailbox.request.is_none() && !worker_stopped.load(Ordering::Acquire) {
                            mailbox = changed
                                .wait(mailbox)
                                .unwrap_or_else(|error| error.into_inner());
                        }
                        if worker_stopped.load(Ordering::Acquire) {
                            break;
                        }
                        (
                            mailbox.request.take().expect("request checked"),
                            mem::take(&mut mailbox.refresh),
                        )
                    };
                    if refresh || cache.is_none() {
                        generation = generation.saturating_add(1);
                        cache = Some(catalog.discover(generation, &worker_stopped));
                    }
                    if worker_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let (candidates, status) =
                        match cache.as_ref().expect("first request initializes cache") {
                            Ok((candidates, status)) => (
                                search_skill_reference_candidates(candidates, request.query()),
                                status.clone(),
                            ),
                            Err(error) => (
                                Vec::new(),
                                SkillReferenceSearchStatus::Failed(error.clone()),
                            ),
                        };
                    lock.lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .update = Some(SkillReferenceSearchUpdate::final_result(
                        &request, status, candidates,
                    ));
                    worker_readiness.notify();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            shared,
            stopped,
            readiness,
            worker: Some(worker),
        })
    }
}

impl SkillReferenceProvider for LocalSkillReferenceProvider {
    fn search(&mut self, request: SkillReferenceSearchRequest) -> Result<(), String> {
        let mut mailbox = self
            .shared
            .0
            .lock()
            .map_err(|_| "skill worker mailbox poisoned")?;
        mailbox.refresh |= request.refresh_catalog();
        mailbox.request = Some(request);
        self.shared.1.notify_one();
        Ok(())
    }

    fn poll(&mut self) -> Result<SkillReferenceProviderPoll, String> {
        Ok(self
            .shared
            .0
            .lock()
            .map_err(|_| "skill worker mailbox poisoned")?
            .update
            .take()
            .map_or(
                SkillReferenceProviderPoll::Pending,
                SkillReferenceProviderPoll::Update,
            ))
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        if self
            .shared
            .0
            .lock()
            .map_or(true, |mailbox| mailbox.update.is_some())
        {
            return Poll::Ready(());
        }
        self.readiness.poll(context)
    }
}

impl Drop for LocalSkillReferenceProvider {
    fn drop(&mut self) {
        // Pair the stop predicate with the wait mutex to avoid a lost shutdown wakeup.
        let mailbox = self
            .shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.stopped.store(true, Ordering::Release);
        self.shared.1.notify_one();
        drop(mailbox);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
