mod error;

use std::collections::{HashMap, HashSet};

pub use error::RuntimeError;

use crate::{
    AgentBackend, AgentCommand, AgentEngine, AgentEvent, BackendBindingEvidence,
    BackendCommandEvidence, BackendEvent, BackendPoll, BackendResumeTarget, ContinuationStrategy,
    Failure, JournalSequence, ModelReplay, SessionId, SubmissionId, TurnOutcome, TurnRef,
    journal::SessionJournal,
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
    source_anchor_sequence: Option<JournalSequence>,
    accepted_requests: HashMap<TurnRef, JournalSequence>,
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
            source_anchor_sequence: None,
            accepted_requests: HashMap::new(),
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
        if !evidence.is_valid()
            || evidence.backend_kind() != previous.backend_kind()
            || evidence.session_locator() != previous.session_locator()
            || evidence.continuation_strategy() != previous.continuation_strategy()
            || evidence.binding_identity() == previous.binding_identity()
        {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "exact-replay replacement returned an invalid or unchanged binding identity",
            )));
        }
        self.publish_resume_snapshot()?;
        let epoch = target.epoch().checked_add(1).ok_or_else(|| {
            RuntimeError::backend(crate::BackendFailure::new(
                crate::BackendFailureKind::Session,
                "backend binding epoch is exhausted",
            ))
        })?;
        if !self.journal.commit_exact_replay_replacement(
            target.epoch(),
            epoch,
            target.source_anchor_sequence(),
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
        self.source_anchor_sequence = Some(target.source_anchor_sequence());
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
        self.source_anchor_sequence = Some(target.source_anchor_sequence());
        self.accepted_requests.clear();
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
                self.accepted_requests.remove(&submission_turn(&command));
            }
            return Err(error);
        }
        let committed = command.clone();
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
                    evidence,
                );
                self.accepted_requests.insert(turn, accepted);
            },
            (Some(submission_id), BackendCommandEvidence::None) => {
                let inserted = self.submission_ids.insert(submission_id);
                debug_assert!(inserted, "a duplicate submission cannot pass validation");
                self.accepted_requests.remove(&submission_turn(&committed));
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
            },
            (None, BackendCommandEvidence::None) => {
                self.journal.append_committed_command(committed, &events);
            },
            _ => unreachable!("command evidence was validated before semantic commit"),
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
            Ok(BackendPoll::Pending) => Ok(RuntimePoll::Pending),
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
                replay.apply(delta).map(|()| replay)
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
            return match self.engine.finish_turn(turn, TurnOutcome::Completed) {
                Ok(event) => {
                    let anchor = self.journal.append_resumable_turn(
                        &event,
                        epoch,
                        accepted_request_sequence,
                        continuation_strategy,
                        evidence,
                    );
                    if let Some(replay) = next_replay {
                        self.model_replay = replay;
                    }
                    self.source_anchor_sequence = Some(anchor);
                    self.accepted_requests.remove(&turn);
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
        let (Some(session_id), Some(epoch), Some(binding), Some(source_anchor_sequence)) = (
            self.engine.session_id(),
            self.binding_epoch,
            self.binding.clone(),
            self.source_anchor_sequence,
        ) else {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Session,
                    "binding replacement requires one durable continuation target",
                )),
            ));
        };
        if !matches!(
            binding.continuation_strategy(),
            ContinuationStrategy::ExactReplay { .. }
        ) {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Unsupported,
                    "binding replacement requires exact replay",
                )),
            ));
        }
        let target =
            BackendResumeTarget::new(session_id, epoch, binding.clone(), source_anchor_sequence)
                .with_model_replay(self.model_replay.clone());
        let evidence = match candidate.resume_session_replacing_binding(&target) {
            Ok(evidence) => evidence,
            Err(failure) => {
                return Err(reject_replacement_candidate(
                    &mut candidate,
                    RuntimeError::backend(failure),
                ));
            },
        };
        if !evidence.is_valid()
            || evidence.backend_kind() != binding.backend_kind()
            || evidence.session_locator() != binding.session_locator()
            || evidence.continuation_strategy() != binding.continuation_strategy()
            || evidence.binding_identity() == binding.binding_identity()
        {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Session,
                    "exact-replay replacement returned an invalid or unchanged binding identity",
                )),
            ));
        }
        let Some(next_epoch) = epoch.checked_add(1) else {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    crate::BackendFailureKind::Session,
                    "backend binding epoch is exhausted",
                )),
            ));
        };
        if !self.journal.commit_exact_replay_replacement(
            epoch,
            next_epoch,
            source_anchor_sequence,
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

        self.binding_epoch = Some(next_epoch);
        self.binding = Some(evidence.clone());
        self.continuation_strategy = Some(evidence.continuation_strategy());
        let mut previous = std::mem::replace(&mut self.backend, candidate);
        Ok(previous.shutdown().err())
    }
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
    }
}
