use std::collections::VecDeque;

use super::AgentEngine;
use crate::{
    AgentEvent,
    journal::{JournalEntry, SemanticRecord},
};

impl AgentEngine {
    pub(crate) fn from_journal(
        entries: &[JournalEntry],
        supports_steer: bool,
    ) -> Result<Self, String> {
        let mut engine = Self::new();
        let mut expected = VecDeque::new();
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
                | SemanticRecord::ContextCheckpoint(_) => {},
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
