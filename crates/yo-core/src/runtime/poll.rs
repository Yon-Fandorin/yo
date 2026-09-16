use std::slice;

use super::{AgentBackend, AgentRuntime, RuntimeError, RuntimePoll};
use crate::{
    AgentEvent, BackendEvent, BackendFailureKind, BackendPoll, BackendResumeSource,
    ContinuationStrategy, Failure, TurnOutcome,
};

impl<B: AgentBackend> AgentRuntime<B> {
    pub fn poll_event(&mut self) -> Result<RuntimePoll, RuntimeError> {
        if let Some(event) = self.interview_delivery.pop_front() {
            return Ok(RuntimePoll::Event(event));
        }
        if let Some(event) = self.interview_backend.take() {
            return self.apply_backend_event(event);
        }
        self.journal.flush_due();
        match self.backend.poll_event() {
            Ok(BackendPoll::Pending) => {
                if let Some(event) = self.interview_start.take() {
                    self.journal.append_events(slice::from_ref(&event));
                    return Ok(RuntimePoll::Event(event));
                }
                if self.idle_context_checkpoint_committed {
                    self.idle_context_compaction_pending = false;
                    self.idle_context_checkpoint_committed = false;
                }
                Ok(RuntimePoll::Pending)
            },
            Ok(BackendPoll::Closed) if self.engine.active_turn().is_none() => {
                Ok(RuntimePoll::Closed)
            },
            Ok(BackendPoll::Closed) => {
                let failure = crate::BackendFailure::new(
                    BackendFailureKind::ProcessExit,
                    "backend closed while a Turn was active",
                );
                let terminal_events = self.fail_active_turn(&failure);
                Err(RuntimeError::Backend {
                    failure,
                    terminal_events,
                })
            },
            Ok(BackendPoll::Event(event)) => self.apply_backend_event(event),
            Err(failure)
                if self.idle_context_compaction_pending
                    && !self.idle_context_checkpoint_committed
                    && self.engine.active_turn().is_none()
                    && failure.kind() == BackendFailureKind::CommandRejected =>
            {
                // A completed, safely rejected idle summary leaves the prior binding
                // and replay usable. The worker publishes the existing control outcome.
                self.idle_context_compaction_pending = false;
                Err(RuntimeError::backend(failure))
            },
            Err(failure) => {
                let terminal_events = self.fail_active_turn(&failure);
                Err(RuntimeError::Backend {
                    failure,
                    terminal_events,
                })
            },
        }
    }

    /// Releases backend resources and closes any remaining semantic work.
    ///
    /// A successful explicit shutdown interrupts an active Turn. A cleanup failure instead fails
    /// that Turn and retains the generated terminal events in the returned error.
    pub fn shutdown(&mut self) -> Result<Vec<AgentEvent>, RuntimeError> {
        match self.backend.shutdown() {
            Ok(()) => {
                let events = self.engine.interrupt_active_turn();
                self.clear_terminal_correlations(&events);
                self.journal.append_events(&events);
                Ok(events)
            },
            Err(failure) => {
                let terminal_events = self.fail_active_turn(&failure);
                Err(RuntimeError::Backend {
                    failure,
                    terminal_events,
                })
            },
        }
    }

    pub(super) fn apply_backend_event(
        &mut self,
        event: BackendEvent,
    ) -> Result<RuntimePoll, RuntimeError> {
        if let Some(start) = self.interview_start.take() {
            let matching = matches!((&start, &event),
                (AgentEvent::ActivityStarted { activity: first, .. },
                 BackendEvent::ActivityUpdated { activity, update: crate::ActivityUpdate::TextSnapshot(_) }) if first == activity);
            if matching {
                let BackendEvent::ActivityUpdated { activity, update } = event else {
                    unreachable!()
                };
                match self.engine.update_activity(activity, update.clone()) {
                    Ok(updated) => {
                        self.journal
                            .append_events(&[start.clone(), updated.clone()]);
                        self.interview_delivery.push_back(updated);
                    },
                    Err(_) => {
                        // Preserve the started observation before the ordinary update rejection.
                        self.journal.append_events(slice::from_ref(&start));
                        self.interview_backend =
                            Some(BackendEvent::ActivityUpdated { activity, update });
                    },
                }
            } else {
                self.journal.append_events(slice::from_ref(&start));
                self.interview_backend = Some(event);
            }
            return Ok(RuntimePoll::Event(start));
        }
        if let BackendEvent::ContextPolicyChanged { policy } = event.clone() {
            let expected_revision = self
                .context_policy
                .as_ref()
                .map_or(1, |current| current.policy_revision().saturating_add(1));
            if self.engine.active_turn().is_some()
                || self.binding_epoch.is_none()
                || !matches!(
                    self.continuation_strategy,
                    Some(ContinuationStrategy::ExactReplay {
                        executor: crate::ReplayExecutor::LocalClient,
                        ..
                    })
                )
                || policy.policy_revision() != expected_revision
            {
                return self.reject_correlation_event(
                    "backend proposed a context policy outside an idle local-client exact-replay binding",
                );
            }
            if !self.journal.append_context_policy(policy.clone()) {
                return self.reject_correlation_event(
                    "backend context policy could not be committed durably",
                );
            }
            self.context_policy = Some(policy);
            self.context_epoch.get_or_insert(1);
            self.context_policy_initialized = true;
            return Ok(RuntimePoll::Pending);
        }
        if let BackendEvent::ContextCheckpointPrepared { proposal } = event.clone() {
            let Some(policy) = self.context_policy.as_ref() else {
                return self.reject_correlation_event(
                    "backend proposed a context checkpoint without a current policy",
                );
            };
            let Some(epoch) = self.binding_epoch else {
                return self.reject_correlation_event(
                    "backend proposed a context checkpoint without an open binding",
                );
            };
            let Some(previous_context_epoch) = self.context_epoch else {
                return self.reject_correlation_event(
                    "backend proposed a context checkpoint without a context epoch",
                );
            };
            let Some(BackendResumeSource::ContinuationAnchor(source_anchor)) = self.resume_source
            else {
                return self.reject_correlation_event(
                    "backend proposed a context checkpoint without a current continuation Anchor",
                );
            };
            if proposal.turn().is_some() != self.engine.active_turn().is_some()
                || proposal
                    .turn()
                    .is_some_and(|turn| self.engine.active_turn() != Some(turn))
            {
                return self.reject_correlation_event(
                    "backend context checkpoint does not match the current Turn boundary",
                );
            }
            let Some((sequence, replay)) = self.journal.commit_context_checkpoint(
                &proposal,
                policy,
                epoch,
                previous_context_epoch,
                source_anchor,
                self.active_context_source.as_ref(),
            ) else {
                return self.reject_correlation_event(
                    "backend context checkpoint could not be validated and committed durably",
                );
            };
            self.context_epoch = previous_context_epoch.checked_add(1);
            self.context_replay_groups = vec![replay.items().to_vec()];
            self.model_replay = replay;
            self.replay_contract_rebind_required = false;
            self.resume_source = Some(BackendResumeSource::ContextCheckpoint(sequence));
            self.active_context_source = None;
            if proposal.turn().is_none() && self.idle_context_compaction_pending {
                self.idle_context_checkpoint_committed = true;
            }
            return Ok(RuntimePoll::Pending);
        }
        if let BackendEvent::ContextActiveSuffixCompleted { turn, items } = event.clone() {
            let Some(last_sequence) = self.journal.last_sequence() else {
                return self.reject_correlation_event(
                    "backend completed an active context suffix without Journal evidence",
                );
            };
            if self.engine.active_turn() != Some(turn)
                || self.engine.active_turn_has_open_activity()
                || !self
                    .active_context_source
                    .as_mut()
                    .is_some_and(|source| source.try_advance(turn, last_sequence, items))
            {
                return self.reject_correlation_event(
                    "backend completed an active context suffix outside a closed semantic boundary",
                );
            }
            return Ok(RuntimePoll::Pending);
        }
        if let BackendEvent::ModelRequestAccepted { turn, evidence } = event.clone() {
            if self.engine.active_turn() != Some(turn) || !evidence.is_valid() {
                return self.reject_correlation_event(
                    "backend accepted a post-checkpoint request outside its active Turn",
                );
            }
            let (Some(epoch), Some(context_epoch)) = (self.binding_epoch, self.context_epoch)
            else {
                return self.reject_correlation_event(
                    "backend accepted a post-checkpoint request without durable correlation state",
                );
            };
            let accepted =
                self.journal
                    .append_accepted_request(turn, epoch, context_epoch, evidence);
            self.accepted_requests.insert(turn, accepted);
            self.binding_has_accepted_request = true;
            self.binding_has_unanchored_request = true;
            return Ok(RuntimePoll::Pending);
        }
        if let BackendEvent::ResumableTurnFinished { turn, evidence } = event.clone() {
            let Some(continuation_strategy) = self.continuation_strategy else {
                return self.reject_correlation_event(
                    "backend completed a resumable Turn without a continuation strategy",
                );
            };
            let replay_matches_strategy = match continuation_strategy {
                ContinuationStrategy::ExactReplay { .. } => evidence.model_replay().is_some(),
                ContinuationStrategy::BackendManagedState => evidence.model_replay().is_none(),
            };
            if !evidence.is_valid() || !replay_matches_strategy {
                return self.reject_correlation_event(
                    "backend completed a resumable Turn with evidence incompatible with its continuation strategy",
                );
            }
            let Some(epoch) = self.binding_epoch else {
                return self.reject_correlation_event(
                    "backend completed a resumable Turn without an open binding",
                );
            };
            let Some(accepted_request_sequence) = self.accepted_requests.get(&turn).copied() else {
                return self.reject_correlation_event(
                    "backend completed a resumable Turn without an accepted request",
                );
            };
            let next_replay = evidence.model_replay().map(|delta| {
                let mut replay = self.model_replay.clone();
                let applied = if self.replay_contract_rebind_required {
                    replay.apply_binding_replacement(delta)
                } else {
                    replay.apply(delta)
                };
                applied.map(|()| replay)
            });
            let next_replay = match next_replay {
                Some(Ok(replay)) => Some(replay),
                Some(Err(_)) => {
                    return self.reject_correlation_event(
                        "backend completed a resumable Turn with an invalid replay delta",
                    );
                },
                None => None,
            };
            let completed_group = evidence.model_replay().map(|delta| delta.items().to_vec());
            return match self.engine.finish_turn(turn, TurnOutcome::Completed) {
                Ok(event) => {
                    let anchor = self.journal.append_resumable_turn(
                        &event,
                        epoch,
                        self.context_epoch,
                        accepted_request_sequence,
                        continuation_strategy,
                        evidence,
                    );
                    if let Some(replay) = next_replay {
                        self.model_replay = replay;
                        self.replay_contract_rebind_required = false;
                    }
                    if self.context_epoch.is_some()
                        && let Some(group) = completed_group
                    {
                        self.context_replay_groups.push(group);
                    }
                    self.resume_source = Some(BackendResumeSource::ContinuationAnchor(anchor));
                    self.binding_has_unanchored_request = false;
                    self.accepted_requests.remove(&turn);
                    self.accepted_submissions.remove(&turn);
                    self.active_context_source = None;
                    Ok(RuntimePoll::Event(event))
                },
                Err(rejection) => {
                    let failure = crate::BackendFailure::new(
                        BackendFailureKind::Protocol,
                        format!("backend event violated core state: {rejection}"),
                    );
                    let terminal_events = self.fail_active_turn(&failure);
                    Err(RuntimeError::EventRejected {
                        event: Box::new(event),
                        rejection,
                        terminal_events,
                    })
                },
            };
        }

        let result = match event.clone() {
            BackendEvent::ActivityStarted { activity, kind } => {
                self.engine.start_activity(activity, kind)
            },
            BackendEvent::ActivityUpdated { activity, update } => {
                self.engine.update_activity(activity, update)
            },
            BackendEvent::ActivityFinished { activity, outcome } => {
                self.engine.finish_activity(activity, outcome)
            },
            BackendEvent::TurnFinished { turn, outcome } => self.engine.finish_turn(turn, outcome),
            BackendEvent::ContextPolicyChanged { .. } => {
                unreachable!("context policy is handled before generic events")
            },
            BackendEvent::ContextCheckpointPrepared { .. } => {
                unreachable!("context checkpoint is handled before generic events")
            },
            BackendEvent::ContextActiveSuffixCompleted { .. } => {
                unreachable!("active context suffix is handled before generic events")
            },
            BackendEvent::ModelRequestAccepted { .. } => {
                unreachable!("request acceptance is handled before generic events")
            },
            BackendEvent::ResumableTurnFinished { .. } => {
                unreachable!("resumable completion is handled before generic events")
            },
        };

        match result {
            Ok(event) => {
                if matches!(
                    event,
                    AgentEvent::ActivityStarted {
                        kind: crate::ActivityKind::UserInputRequest { .. },
                        ..
                    }
                ) {
                    self.interview_start = Some(event);
                    return match self.backend.poll_event() {
                        Ok(BackendPoll::Event(next)) => self.apply_backend_event(next),
                        Ok(BackendPoll::Pending | BackendPoll::Closed) => {
                            let start = self.interview_start.take().expect("held request start");
                            self.journal.append_events(slice::from_ref(&start));
                            Ok(RuntimePoll::Event(start))
                        },
                        Err(failure) => {
                            let terminal_events = self.fail_active_turn(&failure);
                            Err(RuntimeError::Backend {
                                failure,
                                terminal_events,
                            })
                        },
                    };
                }
                self.clear_terminal_correlations(slice::from_ref(&event));
                self.journal.append_events(slice::from_ref(&event));
                Ok(RuntimePoll::Event(event))
            },
            Err(rejection) => {
                let failure = crate::BackendFailure::new(
                    BackendFailureKind::Protocol,
                    format!("backend event violated core state: {rejection}"),
                );
                let terminal_events = self.fail_active_turn(&failure);
                Err(RuntimeError::EventRejected {
                    event: Box::new(event),
                    rejection,
                    terminal_events,
                })
            },
        }
    }

    fn reject_correlation_event(
        &mut self,
        message: &'static str,
    ) -> Result<RuntimePoll, RuntimeError> {
        let failure = crate::BackendFailure::new(BackendFailureKind::Protocol, message);
        let terminal_events = self.fail_active_turn(&failure);
        Err(RuntimeError::Backend {
            failure,
            terminal_events,
        })
    }

    fn fail_active_turn(&mut self, failure: &crate::BackendFailure) -> Vec<AgentEvent> {
        if let Some(start) = self.interview_start.take() {
            self.journal.append_events(slice::from_ref(&start));
        }
        let events = self
            .engine
            .fail_active_turn(Failure::new(failure.to_string()));
        self.clear_terminal_correlations(&events);
        self.journal.append_events(&events);
        events
    }

    fn clear_terminal_correlations(&mut self, events: &[AgentEvent]) {
        for event in events {
            if let AgentEvent::TurnFinished { turn, .. } = event {
                self.accepted_requests.remove(turn);
                self.accepted_submissions.remove(turn);
                self.active_context_source = None;
            }
        }
    }
}
