mod error;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

pub use error::RuntimeError;

use crate::{
    AgentBackend, AgentCommand, AgentEngine, AgentEvent, BackendBindingEvidence,
    BackendCommandEvidence, BackendEvent, BackendPoll, BackendResumeSource, BackendResumeTarget,
    ContextPolicyChanged, ContinuationStrategy, Failure, JournalSequence, ModelReplay, SessionId,
    SubmissionId, TurnOutcome, TurnRef,
    journal::{ContextActiveSource, SessionJournal},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimePoll {
    Pending,
    Event(AgentEvent),
    Closed,
}

pub(crate) struct BackendReplacementError {
    pub(crate) primary: RuntimeError,
    pub(crate) cleanup_failure: Option<crate::BackendFailure>,
}

/// Composes one semantic engine with one initialized backend.
pub struct AgentRuntime<B> {
    engine: AgentEngine,
    backend: B,
    journal: SessionJournal,
    submission_ids: HashSet<SubmissionId>,
    binding_epoch: Option<u64>,
    binding: Option<BackendBindingEvidence>,
    continuation_strategy: Option<ContinuationStrategy>,
    model_replay: ModelReplay,
    replay_contract_rebind_required: bool,
    resume_source: Option<BackendResumeSource>,
    binding_has_accepted_request: bool,
    binding_has_unanchored_request: bool,
    context_policy: Option<ContextPolicyChanged>,
    context_epoch: Option<u64>,
    context_policy_initialized: bool,
    idle_context_compaction_pending: bool,
    idle_context_checkpoint_committed: bool,
    context_replay_groups: Vec<Vec<crate::ModelReplayItem>>,
    active_context_source: Option<ContextActiveSource>,
    accepted_requests: HashMap<TurnRef, JournalSequence>,
    accepted_submissions: HashMap<TurnRef, SubmissionId>,
}

impl<B: AgentBackend> AgentRuntime<B> {
    pub fn new(backend: B) -> Self {
        Self::with_journal(backend, SessionJournal::new())
    }

    pub(crate) fn with_journal(backend: B, journal: SessionJournal) -> Self {
        Self {
            engine: AgentEngine::new(),
            backend,
            journal,
            submission_ids: HashSet::new(),
            binding_epoch: None,
            binding: None,
            continuation_strategy: None,
            model_replay: ModelReplay::default(),
            replay_contract_rebind_required: false,
            resume_source: None,
            binding_has_accepted_request: false,
            binding_has_unanchored_request: false,
            context_policy: None,
            context_epoch: None,
            context_policy_initialized: false,
            idle_context_compaction_pending: false,
            idle_context_checkpoint_committed: false,
            context_replay_groups: Vec::new(),
            active_context_source: None,
            accepted_requests: HashMap::new(),
            accepted_submissions: HashMap::new(),
        }
    }

    pub(crate) fn initialize_resume(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<(), RuntimeError> {
        self.restore_resume_state(target)?;
        let evidence = self
            .backend
            .resume_session(target)
            .map_err(RuntimeError::backend)?;
        let expected = target.binding();
        if !expected.same_resume_identity(&evidence) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "native resume returned a binding identity different from the durable Continuation Anchor",
            )));
        }
        self.publish_resume_snapshot()?;
        Ok(())
    }

    pub(crate) fn initialize_resume_replacing_binding(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<(), RuntimeError> {
        self.restore_resume_state(target)?;
        if !matches!(
            target.binding().continuation_strategy(),
            ContinuationStrategy::ExactReplay { .. }
        ) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Unsupported,
                "binding replacement requires an exact-replay Continuation Anchor",
            )));
        }
        let evidence = self
            .backend
            .resume_session_replacing_binding(target)
            .map_err(RuntimeError::backend)?;
        let previous = target.binding();
        if !valid_replacement_binding(previous, &evidence, target.model_replay()) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "exact-replay replacement returned an incompatible target binding",
            )));
        }
        self.publish_resume_snapshot()?;
        let epoch = target.epoch().checked_add(1).ok_or_else(|| {
            RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "backend binding epoch is exhausted",
            ))
        })?;
        let source = target.source().ok_or_else(|| {
            RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "exact-replay replacement requires one durable source",
            ))
        })?;
        if !self.journal.commit_exact_replay_replacement(
            target.epoch(),
            epoch,
            source,
            evidence.clone(),
        ) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "exact-replay replacement could not publish its binding transition",
            )));
        }
        self.binding_epoch = Some(epoch);
        self.binding = Some(evidence.clone());
        self.continuation_strategy = Some(evidence.continuation_strategy());
        self.model_replay = target.model_replay().clone();
        if evidence.continuation_strategy() == ContinuationStrategy::BackendManagedState {
            self.model_replay = ModelReplay::default();
            self.replay_contract_rebind_required = false;
        } else {
            self.replay_contract_rebind_required = true;
        }
        self.resume_source = Some(source);
        self.binding_has_accepted_request = false;
        self.binding_has_unanchored_request = false;
        Ok(())
    }

    fn restore_resume_state(&mut self, target: &BackendResumeTarget) -> Result<(), RuntimeError> {
        let entries = self.journal.semantic_entries();
        self.engine =
            AgentEngine::from_journal(&entries, self.backend.capabilities().supports_steer())
                .map_err(|detail| {
                    RuntimeError::backend(crate::BackendFailure::new(
                        crate::BackendFailureKind::Protocol,
                        detail,
                    ))
                })?;
        self.submission_ids = entries
            .iter()
            .filter_map(|entry| match entry.record() {
                crate::journal::SemanticRecord::CommandCommitted(committed) => {
                    committed.submission_id()
                },
                _ => None,
            })
            .collect();
        self.binding_epoch = Some(target.epoch());
        self.binding = Some(target.binding().clone());
        self.continuation_strategy = Some(target.binding().continuation_strategy());
        self.model_replay = target.model_replay().clone();
        self.replay_contract_rebind_required = target.replay_contract_rebind_required();
        self.resume_source = target.source();
        self.binding_has_accepted_request = target.binding_has_accepted_request();
        self.binding_has_unanchored_request = false;
        self.context_policy = target.context_policy().cloned();
        self.context_epoch = target.context_epoch();
        self.context_policy_initialized = true;
        self.context_replay_groups = target.model_replay_groups().to_vec();
        self.active_context_source = None;
        self.accepted_requests.clear();
        self.accepted_submissions.clear();
        Ok(())
    }

    fn publish_resume_snapshot(&mut self) -> Result<(), RuntimeError> {
        self.journal.initialize_durability();
        if !matches!(self.durability(), crate::JournalDurability::Durable { .. }) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "native resume could not publish its required complete Journal snapshot",
            )));
        }
        Ok(())
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.engine.session_id()
    }

    pub fn active_turn(&self) -> Option<TurnRef> {
        self.engine.active_turn()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub(crate) fn durability(&self) -> crate::JournalDurability {
        self.journal.transcript_reader().durability()
    }

    pub(crate) const fn idle_context_compaction_pending(&self) -> bool {
        self.idle_context_compaction_pending
    }

    pub(crate) fn initialize_durability(&mut self) {
        self.journal.initialize_durability();
    }

    /// Validates a command, lets the backend accept it, then commits its semantic transition.
    pub fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        if matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        ) {
            return Err(RuntimeError::SubmissionIdentityRequired);
        }
        self.execute(command, None)
    }

    pub fn execute_submission(
        &mut self,
        command: AgentCommand,
        submission_id: SubmissionId,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        if !matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        ) {
            return Err(RuntimeError::SubmissionIdentityUnexpected);
        }
        if self.submission_ids.contains(&submission_id) {
            return Err(RuntimeError::DuplicateSubmissionIdentity(submission_id));
        }
        self.execute(command, Some(submission_id))
    }

    fn execute(
        &mut self,
        command: AgentCommand,
        submission_id: Option<SubmissionId>,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        if matches!(command, AgentCommand::StartTurn { .. })
            && !self.context_policy_initialized
            && matches!(
                self.continuation_strategy,
                Some(ContinuationStrategy::ExactReplay {
                    executor: crate::ReplayExecutor::LocalClient,
                    ..
                })
            )
        {
            match self.backend.poll_event().map_err(RuntimeError::backend)? {
                BackendPoll::Event(event @ BackendEvent::ContextPolicyChanged { .. }) => {
                    self.apply_backend_event(event)?;
                },
                BackendPoll::Event(_) | BackendPoll::Pending | BackendPoll::Closed => {
                    return Err(RuntimeError::backend(crate::BackendFailure::new(
                        crate::BackendFailureKind::Protocol,
                        "local-client exact replay did not publish its context policy before the first model request",
                    )));
                },
            }
        }
        let supports_steer = self.backend.capabilities().supports_steer();
        self.engine
            .validate_command(&command, supports_steer)
            .map_err(RuntimeError::CommandRejected)?;
        let evidence = self
            .backend
            .execute_command(command.clone())
            .map_err(RuntimeError::backend)?;
        if let Err(error) = self.validate_command_evidence(&command, submission_id, &evidence) {
            if submission_id.is_some() {
                let turn = submission_turn(&command);
                self.accepted_requests.remove(&turn);
                self.accepted_submissions.remove(&turn);
            }
            return Err(error);
        }
        let committed = command.clone();
        let starts_idle_context_compaction =
            matches!(&committed, AgentCommand::CompactContext { .. });
        let command_sequence = self.journal.next_sequence();
        let active_input = match &committed {
            AgentCommand::StartTurn { turn, input } => Some((
                *turn,
                crate::ModelReplayItem::Message {
                    role: crate::ModelReplayRole::User,
                    content: input.as_str().to_owned(),
                    refusal: None,
                },
            )),
            _ => None,
        };
        let events = self
            .engine
            .commit_command(command, supports_steer)
            .map_err(RuntimeError::StateDiverged)?;
        match (submission_id, evidence) {
            (Some(submission_id), BackendCommandEvidence::RequestAccepted(evidence)) => {
                let inserted = self.submission_ids.insert(submission_id);
                debug_assert!(inserted, "a duplicate submission cannot pass validation");
                let epoch = self
                    .binding_epoch
                    .expect("request evidence validation requires an open binding");
                let turn = submission_turn(&committed);
                let accepted = self.journal.append_accepted_submission(
                    committed,
                    submission_id,
                    &events,
                    epoch,
                    self.context_epoch,
                    evidence,
                );
                self.accepted_requests.insert(turn, accepted);
                self.accepted_submissions.insert(turn, submission_id);
                self.binding_has_accepted_request = true;
                self.binding_has_unanchored_request = true;
            },
            (Some(submission_id), BackendCommandEvidence::None) => {
                let inserted = self.submission_ids.insert(submission_id);
                debug_assert!(inserted, "a duplicate submission cannot pass validation");
                let turn = submission_turn(&committed);
                self.accepted_requests.remove(&turn);
                self.accepted_submissions.insert(turn, submission_id);
                self.journal
                    .append_committed_submission(committed, submission_id, &events);
            },
            (None, BackendCommandEvidence::BindingOpened(evidence)) => {
                let epoch = 1;
                self.binding = Some(evidence.clone());
                self.continuation_strategy = Some(evidence.continuation_strategy());
                self.journal
                    .append_initial_binding(committed, &events, epoch, evidence);
                self.binding_epoch = Some(epoch);
                self.binding_has_accepted_request = false;
                self.binding_has_unanchored_request = false;
            },
            (None, BackendCommandEvidence::None) => {
                self.journal.append_committed_command(committed, &events);
            },
            _ => unreachable!("command evidence was validated before semantic commit"),
        }
        if starts_idle_context_compaction {
            self.idle_context_compaction_pending = true;
            self.idle_context_checkpoint_committed = false;
        }
        if let Some((turn, item)) = active_input {
            self.active_context_source = Some(ContextActiveSource::new(
                turn,
                command_sequence,
                command_sequence,
                vec![item],
            ));
        }
        Ok(events)
    }

    fn validate_command_evidence(
        &self,
        command: &AgentCommand,
        submission_id: Option<SubmissionId>,
        evidence: &BackendCommandEvidence,
    ) -> Result<(), RuntimeError> {
        let valid = match evidence {
            BackendCommandEvidence::None => true,
            BackendCommandEvidence::BindingOpened(evidence) => {
                matches!(command, AgentCommand::CreateSession { .. })
                    && submission_id.is_none()
                    && self.binding_epoch.is_none()
                    && evidence.is_valid()
            },
            BackendCommandEvidence::RequestAccepted(evidence) => {
                matches!(
                    command,
                    AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
                ) && submission_id.is_some()
                    && self.binding_epoch.is_some()
                    && evidence.is_valid()
            },
        };
        if valid {
            return Ok(());
        }
        Err(RuntimeError::backend(crate::BackendFailure::new(
            crate::BackendFailureKind::Protocol,
            format!(
                "backend returned correlation evidence incompatible with {}",
                command_kind(command)
            ),
        )))
    }

    /// Applies one available backend observation through the semantic engine.
    pub fn poll_event(&mut self) -> Result<RuntimePoll, RuntimeError> {
        self.journal.flush_due();
        match self.backend.poll_event() {
            Ok(BackendPoll::Pending) => {
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
                    crate::BackendFailureKind::ProcessExit,
                    "backend closed while a Turn was active",
                );
                let terminal_events = self.fail_active_turn(&failure);
                Err(RuntimeError::Backend {
                    failure,
                    terminal_events,
                })
            },
            Ok(BackendPoll::Event(event)) => self.apply_backend_event(event),
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

    fn apply_backend_event(&mut self, event: BackendEvent) -> Result<RuntimePoll, RuntimeError> {
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
                        crate::BackendFailureKind::Protocol,
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
                self.clear_terminal_correlations(std::slice::from_ref(&event));
                self.journal.append_events(std::slice::from_ref(&event));
                Ok(RuntimePoll::Event(event))
            },
            Err(rejection) => {
                let failure = crate::BackendFailure::new(
                    crate::BackendFailureKind::Protocol,
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
        let failure = crate::BackendFailure::new(crate::BackendFailureKind::Protocol, message);
        let terminal_events = self.fail_active_turn(&failure);
        Err(RuntimeError::Backend {
            failure,
            terminal_events,
        })
    }

    fn fail_active_turn(&mut self, failure: &crate::BackendFailure) -> Vec<AgentEvent> {
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

    #[cfg(test)]
    pub(crate) fn journal(&self) -> &SessionJournal {
        &self.journal
    }
}

impl AgentRuntime<Box<dyn AgentBackend + Send>> {
    /// Replaces one idle exact-replay binding without releasing the current backend first.
    ///
    /// The candidate is resumed and its transition is durably committed before it becomes active.
    /// Any pre-commit failure cleans up only the candidate, leaving the current backend usable.
    pub(crate) fn replace_backend(
        &mut self,
        mut candidate: Box<dyn AgentBackend + Send>,
    ) -> Result<Option<crate::BackendFailure>, BackendReplacementError> {
        if self.engine.active_turn().is_some() {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Session,
                    "binding replacement requires an idle Session",
                )),
            ));
        }
        let (Some(session_id), Some(epoch), Some(binding)) = (
            self.engine.session_id(),
            self.binding_epoch,
            self.binding.clone(),
        ) else {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Session,
                    "binding replacement requires one open durable binding",
                )),
            ));
        };
        let Some(next_epoch) = epoch.checked_add(1) else {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Session,
                    "backend binding epoch is exhausted",
                )),
            ));
        };

        let evidence = match binding.continuation_strategy() {
            ContinuationStrategy::ExactReplay { .. } => {
                let Some(resume_source) = self.resume_source else {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            crate::BackendFailureKind::Session,
                            "exact-replay replacement requires one durable continuation source",
                        )),
                    ));
                };
                let target = match resume_source {
                    BackendResumeSource::ContinuationAnchor(sequence) => {
                        BackendResumeTarget::new(session_id, epoch, binding.clone(), sequence)
                    },
                    BackendResumeSource::ContextCheckpoint(sequence) => {
                        BackendResumeTarget::from_checkpoint(
                            session_id,
                            epoch,
                            binding.clone(),
                            sequence,
                        )
                    },
                }
                .with_model_replay(self.model_replay.clone())
                .with_context_state(
                    self.context_policy.clone(),
                    self.context_epoch,
                    self.context_replay_groups.clone(),
                );
                let evidence = candidate
                    .resume_session_replacing_binding(&target)
                    .map_err(RuntimeError::backend)
                    .map_err(|primary| reject_replacement_candidate(&mut candidate, primary))?;
                if !valid_replacement_binding(&binding, &evidence, &self.model_replay) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            crate::BackendFailureKind::Session,
                            "exact-replay replacement returned an incompatible target binding",
                        )),
                    ));
                }
                if !self.journal.commit_exact_replay_replacement(
                    epoch,
                    next_epoch,
                    resume_source,
                    evidence.clone(),
                ) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            crate::BackendFailureKind::Session,
                            "exact-replay replacement could not publish its binding transition",
                        )),
                    ));
                }
                evidence
            },
            ContinuationStrategy::BackendManagedState => {
                if !self.backend.capabilities().supports_native_model_rebind()
                    || !candidate.capabilities().supports_native_model_rebind()
                {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            crate::BackendFailureKind::Unsupported,
                            "source and candidate backends must advertise native model rebinding",
                        )),
                    ));
                }
                let source_anchor = if self.binding_has_accepted_request {
                    if self.binding_has_unanchored_request {
                        return Err(reject_replacement_candidate(
                            &mut candidate,
                            RuntimeError::backend(crate::BackendFailure::new(
                                crate::BackendFailureKind::Session,
                                "native model rebinding requires the newest durable continuation Anchor after an accepted request",
                            )),
                        ));
                    }
                    match self.resume_source {
                        Some(BackendResumeSource::ContinuationAnchor(sequence)) => Some(sequence),
                        Some(BackendResumeSource::ContextCheckpoint(_)) | None => {
                            return Err(reject_replacement_candidate(
                                &mut candidate,
                                RuntimeError::backend(crate::BackendFailure::new(
                                    crate::BackendFailureKind::Session,
                                    "native model rebinding requires the newest durable continuation Anchor after an accepted request",
                                )),
                            ));
                        },
                    }
                } else {
                    None
                };
                let target = BackendResumeTarget::for_model_rebind(
                    session_id,
                    epoch,
                    binding.clone(),
                    source_anchor,
                );
                let evidence = candidate
                    .resume_session_rebinding_model(&target)
                    .map_err(RuntimeError::backend)
                    .map_err(|primary| reject_replacement_candidate(&mut candidate, primary))?;
                if !valid_native_model_rebind(&binding, &evidence) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            crate::BackendFailureKind::Session,
                            "native model rebind returned an incompatible target binding",
                        )),
                    ));
                }
                if !self.journal.commit_native_model_rebind(
                    epoch,
                    next_epoch,
                    source_anchor,
                    evidence.clone(),
                ) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            crate::BackendFailureKind::Session,
                            "native model rebind could not publish its binding transition",
                        )),
                    ));
                }
                self.resume_source = source_anchor.map(BackendResumeSource::ContinuationAnchor);
                self.binding_has_accepted_request = false;
                self.binding_has_unanchored_request = false;
                evidence
            },
        };

        self.binding_epoch = Some(next_epoch);
        self.binding = Some(evidence.clone());
        self.continuation_strategy = Some(evidence.continuation_strategy());
        self.binding_has_accepted_request = false;
        self.binding_has_unanchored_request = false;
        if evidence.continuation_strategy() == ContinuationStrategy::BackendManagedState {
            self.model_replay = ModelReplay::default();
            self.context_replay_groups.clear();
            self.replay_contract_rebind_required = false;
        } else {
            self.replay_contract_rebind_required = true;
        }
        let mut previous = std::mem::replace(&mut self.backend, candidate);
        Ok(previous.shutdown().err())
    }
}

fn valid_replacement_binding(
    previous: &BackendBindingEvidence,
    replacement: &BackendBindingEvidence,
    replay: &ModelReplay,
) -> bool {
    if !replacement.is_valid()
        || replacement.backend_kind() != previous.backend_kind()
        || replacement.session_locator() != previous.session_locator()
    {
        return false;
    }
    let has_provider_private = replay.items().iter().any(|item| {
        matches!(
            item,
            crate::ModelReplayItem::ProviderPrivateAssistant { .. }
        )
    });
    if !has_provider_private {
        return replacement.binding_identity() != previous.binding_identity();
    }
    let replay_profile = |strategy| match strategy {
        ContinuationStrategy::ExactReplay { replay_profile, .. } => Some(replay_profile),
        ContinuationStrategy::BackendManagedState => None,
    };
    replacement.binding_identity() == previous.binding_identity()
        && replay_profile(replacement.continuation_strategy())
            == replay_profile(previous.continuation_strategy())
}

fn valid_native_model_rebind(
    previous: &BackendBindingEvidence,
    replacement: &BackendBindingEvidence,
) -> bool {
    replacement.is_valid()
        && previous.continuation_strategy() == ContinuationStrategy::BackendManagedState
        && replacement.continuation_strategy() == ContinuationStrategy::BackendManagedState
        && replacement.backend_kind() == previous.backend_kind()
        && replacement.binding_identity() != previous.binding_identity()
        && replacement.session_locator() != previous.session_locator()
        && replacement.model_identity() != previous.model_identity()
}

fn reject_replacement_candidate(
    candidate: &mut Box<dyn AgentBackend + Send>,
    primary: RuntimeError,
) -> BackendReplacementError {
    BackendReplacementError {
        primary,
        cleanup_failure: candidate.shutdown().err(),
    }
}

fn submission_turn(command: &AgentCommand) -> TurnRef {
    match command {
        AgentCommand::StartTurn { turn, .. } | AgentCommand::SteerTurn { turn, .. } => *turn,
        _ => unreachable!("only a submission command has an accepted request"),
    }
}

fn command_kind(command: &AgentCommand) -> &'static str {
    match command {
        AgentCommand::CreateSession { .. } => "CreateSession",
        AgentCommand::StartTurn { .. } => "StartTurn",
        AgentCommand::SteerTurn { .. } => "SteerTurn",
        AgentCommand::InterruptTurn { .. } => "InterruptTurn",
        AgentCommand::RespondToActivity { .. } => "RespondToActivity",
        AgentCommand::CompactContext { .. } => "CompactContext",
    }
}
