use std::collections::VecDeque;

use super::AgentEngine;
use crate::{
    AgentCommand, AgentEvent, SessionId,
    journal::{JournalEntry, SemanticRecord, codec::TransitionMode},
};

impl AgentEngine {
    pub(crate) fn from_journal(
        entries: &[JournalEntry],
        supports_steer: bool,
    ) -> Result<Self, String> {
        let mut engine = Self::new();
        let mut expected = VecDeque::new();
        if let Some(session_id) = initial_fork_session(entries) {
            // The codec validates the atomic bootstrap; only local engine state is restored here.
            expected.extend(
                engine
                    .commit_command(AgentCommand::CreateSession { session_id }, supports_steer)
                    .map_err(|error| format!("fork Session cannot be restored: {error}"))?,
            );
        }
        for entry in entries {
            match entry.record().clone() {
                SemanticRecord::CommandCommitted(committed) => {
                    let events = engine
                        .commit_command(committed.command().clone(), supports_steer)
                        .map_err(|error| {
                            format!(
                                "command at Journal sequence {} cannot be restored: {error}",
                                entry.sequence().get()
                            )
                        })?;
                    expected.extend(events);
                },
                SemanticRecord::EventCommitted(event)
                    if expected
                        .front()
                        .is_some_and(|candidate| candidate == &event) =>
                {
                    expected.pop_front();
                },
                SemanticRecord::EventCommitted(event) => {
                    let restored = match event.clone() {
                        AgentEvent::ActivityStarted { activity, kind } => {
                            engine.start_activity(activity, kind)
                        },
                        AgentEvent::ActivityUpdated { activity, update } => {
                            engine.update_activity(activity, update)
                        },
                        AgentEvent::ActivityFinished { activity, outcome } => {
                            engine.finish_activity(activity, outcome)
                        },
                        AgentEvent::TurnFinished { turn, outcome } => {
                            engine.finish_turn(turn, outcome)
                        },
                        AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {
                            return Err(format!(
                                "unexpected lifecycle event at Journal sequence {}",
                                entry.sequence().get()
                            ));
                        },
                    }
                    .map_err(|error| {
                        format!(
                            "event at Journal sequence {} cannot be restored: {error}",
                            entry.sequence().get()
                        )
                    })?;
                    if restored != event {
                        return Err(format!(
                            "event at Journal sequence {} changed during restoration",
                            entry.sequence().get()
                        ));
                    }
                },
                SemanticRecord::BackendExchangeObserved(_)
                | SemanticRecord::BackendBindingOpened(_)
                | SemanticRecord::BackendBindingClosed(_)
                | SemanticRecord::BackendRequestAccepted(_)
                | SemanticRecord::ModelReplayDelta(_)
                | SemanticRecord::BackendResumableOutcome(_)
                | SemanticRecord::ContinuationAnchor(_)
                | SemanticRecord::ContextPolicyChanged(_)
                | SemanticRecord::ContextCheckpoint(_)
                | SemanticRecord::InitialForkSeed(_) => {},
            }
        }
        if !expected.is_empty() {
            return Err("stored Journal ends before a command's semantic events".to_owned());
        }
        if engine.active_turn().is_some() {
            return Err("a native-resume Journal must end without an active Turn".to_owned());
        }
        Ok(engine)
    }
}

fn initial_fork_session(entries: &[JournalEntry]) -> Option<SessionId> {
    let [created, seed, binding, ..] = entries else {
        return None;
    };
    let (
        SemanticRecord::EventCommitted(AgentEvent::SessionCreated { session_id }),
        SemanticRecord::InitialForkSeed(fork),
        SemanticRecord::BackendBindingOpened(opened),
    ) = (created.record(), seed.record(), binding.record())
    else {
        return None;
    };
    (created.sequence() < seed.sequence()
        && seed.sequence() < binding.sequence()
        && opened.epoch() == 1
        && opened.transition().mode() == TransitionMode::InitialFork
        && opened.transition().fork_seed_sequence() == Some(seed.sequence())
        && fork.validate_child(*session_id).is_ok())
    .then_some(*session_id)
}
