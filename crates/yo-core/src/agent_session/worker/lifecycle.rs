use std::{
    collections::VecDeque,
    sync::{
        Condvar, Mutex, PoisonError,
        atomic::{AtomicU8, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    thread,
};

use super::{
    super::{
        AgentControlOutcome, AgentSessionError, BackendReplacementOutcome, PendingCommand,
        WORKER_EXECUTING, WORKER_IDLE, WORKER_POLL_INTERVAL, WORKER_STOPPING,
    },
    AgentWorker, ChangeLane, ReplacementRequest, WorkerExit, cancel_queued_turn_commands,
    context_compaction_rejection, submission_rejection,
};
use crate::{
    AgentCommand, AgentEvent, RuntimePoll, SubmissionOutcome, SubmissionRejection,
    SubmissionRejectionKind,
};

impl AgentWorker {
    /// resume 또는 신규 Session handshake를 실행하고 worker 시작 결과를 반환합니다.
    pub(in crate::agent_session) fn initialize(
        &mut self,
    ) -> Result<Vec<AgentEvent>, AgentSessionError> {
        if let Some(resume) = self.resume.take() {
            let initialized = match resume {
                super::super::ResumeInitialization::SameBinding(target) => {
                    self.runtime.initialize_resume(&target)
                },
                super::super::ResumeInitialization::Replacement(target) => {
                    self.runtime.initialize_resume_replacing_binding(&target)
                },
            };
            return match initialized {
                Ok(()) => Ok(Vec::new()),
                Err(start) => match self.runtime.shutdown() {
                    Ok(_) => Err(AgentSessionError::Runtime(start)),
                    Err(cleanup) => Err(AgentSessionError::StartAndCleanup {
                        start: Box::new(start),
                        cleanup: Box::new(cleanup),
                    }),
                },
            };
        }
        self.runtime.initialize_durability();
        match self.execute(
            AgentCommand::CreateSession {
                session_id: self.session_id,
            },
            None,
        ) {
            Ok(events) => Ok(events),
            Err(start) => match self.runtime.shutdown() {
                Ok(_) => Err(AgentSessionError::Runtime(start)),
                Err(cleanup) => Err(AgentSessionError::StartAndCleanup {
                    start: Box::new(start),
                    cleanup: Box::new(cleanup),
                }),
            },
        }
    }

    /// command, replacement, backend poll, stop, cleanup을 소유하는 runtime loop입니다.
    pub(in crate::agent_session) fn run(
        &mut self,
        commands: Receiver<PendingCommand>,
        urgent_commands: Receiver<PendingCommand>,
        replacements: Receiver<ReplacementRequest>,
        changes: &mut ChangeLane,
        processed: &(Mutex<u64>, Condvar),
        lifecycle: &AtomicU8,
    ) -> WorkerExit {
        let mut deferred_commands = VecDeque::new();
        let mut deferred_urgent = VecDeque::new();
        let mut context_compaction_in_flight = false;
        loop {
            if lifecycle.load(Ordering::Acquire) == WORKER_STOPPING {
                return WorkerExit::from_cleanup(self.runtime.shutdown());
            }

            if !self.runtime.idle_context_compaction_pending() {
                match replacements.try_recv() {
                    Ok(request) => {
                        if lifecycle
                            .compare_exchange(
                                WORKER_IDLE,
                                WORKER_EXECUTING,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_err()
                        {
                            let mut backend = request.backend;
                            let cleanup = backend.shutdown().err();
                            let primary = AgentSessionError::WorkerUnavailable(
                                "binding replacement requires an idle Session".to_owned(),
                            );
                            let error = match cleanup {
                                Some(cleanup) => AgentSessionError::Multiple {
                                    primary: Box::new(primary),
                                    additional: Box::new(AgentSessionError::BackendCleanup(
                                        cleanup,
                                    )),
                                },
                                None => primary,
                            };
                            let _ = request.result.send(Err(error));
                            continue;
                        }
                        let durability_before = self.runtime.durability();
                        let result = self
                            .runtime
                            .replace_backend(request.backend)
                            .map(|cleanup_failure| BackendReplacementOutcome { cleanup_failure })
                            .map_err(|error| {
                                let primary = AgentSessionError::Runtime(error.primary);
                                match error.cleanup_failure {
                                    Some(cleanup) => AgentSessionError::Multiple {
                                        primary: Box::new(primary),
                                        additional: Box::new(AgentSessionError::BackendCleanup(
                                            cleanup,
                                        )),
                                    },
                                    None => primary,
                                }
                            });
                        lifecycle.store(WORKER_IDLE, Ordering::Release);
                        let changed =
                            self.runtime.durability() != durability_before || result.is_ok();
                        let _ = request.result.send(result);
                        if changed && !changes.changed() {
                            return WorkerExit::from_cleanup(self.runtime.shutdown());
                        }
                        continue;
                    },
                    Err(TryRecvError::Disconnected | TryRecvError::Empty) => {},
                }
            }

            let command = if self.runtime.idle_context_compaction_pending() {
                Err(TryRecvError::Empty)
            } else {
                let urgent = deferred_urgent
                    .pop_front()
                    .map_or_else(|| urgent_commands.try_recv(), Ok);
                match urgent {
                    Ok(command) => Ok(command),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => deferred_commands
                        .pop_front()
                        .map_or_else(|| commands.try_recv(), Ok),
                }
            };
            match command {
                Ok(pending) => {
                    if let AgentCommand::InterruptTurn { turn } = pending.command() {
                        let canceled =
                            cancel_queued_turn_commands(*turn, &commands, &mut deferred_commands)
                                .into_iter()
                                .chain(cancel_queued_turn_commands(
                                    *turn,
                                    &urgent_commands,
                                    &mut deferred_urgent,
                                ));
                        for id in canceled {
                            self.record_submission_outcome(SubmissionOutcome::Rejected {
                                id,
                                rejection: SubmissionRejection::new(
                                    SubmissionRejectionKind::TargetChanged,
                                    "the target Turn was interrupted before this submission ran",
                                ),
                            });
                        }
                    }
                    if lifecycle
                        .compare_exchange(
                            WORKER_IDLE,
                            WORKER_EXECUTING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                    let (command, submission_id) = pending.into_parts();
                    let is_context_compaction =
                        matches!(&command, AgentCommand::CompactContext { .. });
                    let result = self.dispatch(command, submission_id);
                    if let Ok(mut count) = processed.0.lock() {
                        *count += 1;
                        processed.1.notify_all();
                    }
                    let stopping = lifecycle
                        .compare_exchange(
                            WORKER_EXECUTING,
                            WORKER_IDLE,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err();
                    match result {
                        Ok(_) => {
                            if is_context_compaction {
                                context_compaction_in_flight = true;
                            }
                            if let Some(id) = submission_id {
                                self.record_submission_outcome(SubmissionOutcome::Accepted { id });
                            }
                            if !changes.changed() {
                                return WorkerExit::from_cleanup(self.runtime.shutdown());
                            }
                        },
                        Err(error) => {
                            if let Some(id) = submission_id
                                && let AgentSessionError::Runtime(runtime_error) = &error
                                && let Some(rejection) = submission_rejection(runtime_error)
                            {
                                self.record_submission_outcome(SubmissionOutcome::Rejected {
                                    id,
                                    rejection,
                                });
                                if !changes.changed() || stopping {
                                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                                }
                                continue;
                            }
                            if is_context_compaction
                                && let Some(detail) = context_compaction_rejection(&error)
                            {
                                self.context_compaction_pending
                                    .store(false, Ordering::Release);
                                self.control_outcomes
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .push_back(AgentControlOutcome::ContextCompactionRejected {
                                        detail,
                                    });

                                if !changes.changed() || stopping {
                                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                                }
                                continue;
                            }
                            return self.finish_after_failure(error, changes);
                        },
                    }
                    if stopping {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Err(TryRecvError::Disconnected) => {
                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                },
                Err(TryRecvError::Empty) => {},
            }

            let durability_before = self.runtime.durability();
            let poll = self.poll();
            let durability_changed = self.runtime.durability() != durability_before;
            match poll {
                Ok(RuntimePoll::Pending) => {
                    if durability_changed && !changes.changed() {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Ok(RuntimePoll::Event(_)) => {
                    if !changes.changed() {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Ok(RuntimePoll::Closed) => {
                    return match self.runtime.shutdown().map_err(AgentSessionError::Runtime) {
                        Ok(terminal) => {
                            let published = terminal.is_empty() || changes.changed();
                            let _ = published && changes.close();
                            WorkerExit::success()
                        },
                        Err(error) => {
                            let _ = changes.failure(error);
                            WorkerExit::success()
                        },
                    };
                },
                Err(error) => {
                    let primary = AgentSessionError::Runtime(error);
                    if context_compaction_in_flight
                        && !self.runtime.idle_context_compaction_pending()
                        && let Some(detail) = context_compaction_rejection(&primary)
                    {
                        context_compaction_in_flight = false;
                        self.context_compaction_pending
                            .store(false, Ordering::Release);
                        self.control_outcomes
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push_back(AgentControlOutcome::ContextCompactionRejected { detail });
                        if !changes.changed() {
                            return WorkerExit::from_cleanup(self.runtime.shutdown());
                        }
                        continue;
                    }
                    return self.finish_after_failure(primary, changes);
                },
            }

            if context_compaction_in_flight && !self.runtime.idle_context_compaction_pending() {
                context_compaction_in_flight = false;
                self.context_compaction_pending
                    .store(false, Ordering::Release);
                if !changes.changed() {
                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                }
            }

            thread::sleep(WORKER_POLL_INTERVAL);
        }
    }
}
