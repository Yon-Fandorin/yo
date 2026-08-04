mod error;

use std::collections::HashSet;

pub use error::RuntimeError;

use crate::{
    AgentBackend, AgentCommand, AgentEngine, AgentEvent, BackendEvent, BackendPoll, Failure,
    SessionId, SubmissionId, TurnRef, journal::SessionJournal,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimePoll {
    Pending,
    Event(AgentEvent),
    Closed,
}

/// Composes one semantic engine with one initialized backend.
pub struct AgentRuntime<B> {
    engine: AgentEngine,
    backend: B,
    journal: SessionJournal,
    submission_ids: HashSet<SubmissionId>,
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
        }
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
        self.backend
            .execute_command(command.clone())
            .map_err(RuntimeError::backend)?;
        let committed = command.clone();
        let events = self
            .engine
            .commit_command(command, supports_steer)
            .map_err(RuntimeError::StateDiverged)?;
        if let Some(submission_id) = submission_id {
            let inserted = self.submission_ids.insert(submission_id);
            debug_assert!(inserted, "a duplicate submission cannot pass validation");
            self.journal
                .append_committed_submission(committed, submission_id, &events);
        } else {
            self.journal.append_committed_command(committed, &events);
        }
        Ok(events)
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
        };

        match result {
            Ok(event) => {
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

    fn fail_active_turn(&mut self, failure: &crate::BackendFailure) -> Vec<AgentEvent> {
        let events = self
            .engine
            .fail_active_turn(Failure::new(failure.to_string()));
        self.journal.append_events(&events);
        events
    }

    #[cfg(test)]
    pub(crate) fn journal(&self) -> &SessionJournal {
        &self.journal
    }
}
