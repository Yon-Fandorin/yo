use std::task::{Context, Poll};

use super::LoopError;
use crate::{
    runner::{
        AgentConnection, AgentPoll, SkillReferenceConnection, SkillReferencePoll,
        TerminationSource, WorkspaceReferenceConnection, WorkspaceReferencePoll,
        source_schedule::{OrdinarySource, SourceSchedule},
        state::{StateEffect, StateError, TuiState},
    },
    terminal::backend::unix::{EventSource, UnixEventReader},
};

pub(in crate::runner) enum OrdinaryObservation {
    Input(crate::input::event::InputEvent),
    Agent(AgentPoll),
    Workspace(Result<WorkspaceReferencePoll, String>),
    Skill(Result<SkillReferencePoll, String>),
}

enum ReferenceSourcePoll<P> {
    Pending,
    Reselect,
    Termination,
    Ready(Result<P, String>),
}

// Keeping one bounded observation inline avoids a heap allocation in the owner-thread hot path.
#[allow(clippy::large_enum_variant)]
pub(in crate::runner) enum OrdinaryPoll {
    Pending,
    Reselect,
    Termination,
    Ready {
        source: OrdinarySource,
        observation: OrdinaryObservation,
    },
}

pub(in crate::runner) fn poll_ordinary<E, T, A>(
    events: &mut UnixEventReader<E, T>,
    agent: &mut A,
    workspace_references: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    skill_references: &mut Option<Box<dyn SkillReferenceConnection>>,
    schedule: &SourceSchedule,
    context: &mut Context<'_>,
) -> Result<OrdinaryPoll, LoopError>
where
    E: EventSource,
    E::Error: std::fmt::Debug,
    T: TerminationSource,
    A: AgentConnection,
{
    if events.poll_termination(context).is_ready() {
        return Ok(OrdinaryPoll::Termination);
    }

    let mut must_reselect = false;
    for source in schedule.order() {
        let observation = match source {
            OrdinarySource::Terminal => {
                let polled = events.poll_input(context);
                if events.poll_termination(context).is_ready() {
                    return Ok(OrdinaryPoll::Termination);
                }
                match polled {
                    Poll::Ready(result) => Some(OrdinaryObservation::Input(
                        result.map_err(|error| LoopError::Input(format!("{error:?}")))?,
                    )),
                    Poll::Pending => None,
                }
            },
            OrdinarySource::Agent => {
                let ready = agent.poll_ready(context);
                if events.poll_termination(context).is_ready() {
                    return Ok(OrdinaryPoll::Termination);
                }
                if ready.is_pending() {
                    None
                } else {
                    let polled = agent
                        .poll()
                        .map_err(|error| LoopError::Agent(error.to_string()));
                    if events.poll_termination(context).is_ready() {
                        return Ok(OrdinaryPoll::Termination);
                    }
                    match polled? {
                        AgentPoll::Pending => {
                            must_reselect |= agent.poll_ready(context).is_ready();
                            if events.poll_termination(context).is_ready() {
                                return Ok(OrdinaryPoll::Termination);
                            }
                            None
                        },
                        observation => Some(OrdinaryObservation::Agent(observation)),
                    }
                }
            },
            OrdinarySource::Workspace => {
                let Some(connection) = workspace_references.as_mut() else {
                    continue;
                };
                match poll_reference_source(
                    events,
                    connection,
                    context,
                    |connection, context| connection.poll_ready(context),
                    |connection| connection.poll(),
                    |observation| matches!(observation, WorkspaceReferencePoll::Pending),
                ) {
                    ReferenceSourcePoll::Pending => None,
                    ReferenceSourcePoll::Reselect => {
                        must_reselect = true;
                        None
                    },
                    ReferenceSourcePoll::Termination => return Ok(OrdinaryPoll::Termination),
                    ReferenceSourcePoll::Ready(observation) => {
                        Some(OrdinaryObservation::Workspace(observation))
                    },
                }
            },
            OrdinarySource::Skill => {
                let Some(connection) = skill_references.as_mut() else {
                    continue;
                };
                match poll_reference_source(
                    events,
                    connection,
                    context,
                    |connection, context| connection.poll_ready(context),
                    |connection| connection.poll(),
                    |observation| matches!(observation, SkillReferencePoll::Pending),
                ) {
                    ReferenceSourcePoll::Pending => None,
                    ReferenceSourcePoll::Reselect => {
                        must_reselect = true;
                        None
                    },
                    ReferenceSourcePoll::Termination => return Ok(OrdinaryPoll::Termination),
                    ReferenceSourcePoll::Ready(observation) => {
                        Some(OrdinaryObservation::Skill(observation))
                    },
                }
            },
        };
        if let Some(observation) = observation {
            return Ok(OrdinaryPoll::Ready {
                source,
                observation,
            });
        }
    }

    Ok(if must_reselect {
        OrdinaryPoll::Reselect
    } else {
        OrdinaryPoll::Pending
    })
}

fn poll_reference_source<E, T, C, P, FReady, FPoll, FPending>(
    events: &mut UnixEventReader<E, T>,
    connection: &mut C,
    context: &mut Context<'_>,
    mut poll_ready: FReady,
    poll: FPoll,
    is_pending: FPending,
) -> ReferenceSourcePoll<P>
where
    E: EventSource,
    T: TerminationSource,
    C: ?Sized,
    FReady: FnMut(&mut C, &mut Context<'_>) -> Poll<()>,
    FPoll: FnOnce(&mut C) -> Result<P, String>,
    FPending: FnOnce(&P) -> bool,
{
    let ready = poll_ready(connection, context);
    if events.poll_termination(context).is_ready() {
        return ReferenceSourcePoll::Termination;
    }
    if ready.is_pending() {
        return ReferenceSourcePoll::Pending;
    }

    let observation = poll(connection);
    if events.poll_termination(context).is_ready() {
        return ReferenceSourcePoll::Termination;
    }
    if let Ok(observation) = &observation
        && is_pending(observation)
    {
        let ready_again = poll_ready(connection, context).is_ready();
        if events.poll_termination(context).is_ready() {
            return ReferenceSourcePoll::Termination;
        }
        return if ready_again {
            ReferenceSourcePoll::Reselect
        } else {
            ReferenceSourcePoll::Pending
        };
    }

    ReferenceSourcePoll::Ready(observation)
}

pub(in crate::runner) fn handle_backpressured_input(
    state: &mut TuiState,
    input: crate::input::event::InputEvent,
    now: std::time::Duration,
    allow_pending_request: bool,
) -> Result<StateEffect, StateError> {
    if (allow_pending_request && state.has_pending_request())
        || state.wants_global_input(&input)
        || state.wants_overlay_input(&input)
        || input.is_control_flow_key()
        || matches!(input, crate::input::event::InputEvent::Resize(_))
    {
        state.handle(input, now)
    } else {
        Ok(StateEffect::Unchanged)
    }
}

pub(in crate::runner) fn apply_agent_poll(
    state: &mut TuiState,
    observation: AgentPoll,
) -> Result<bool, LoopError> {
    match observation {
        AgentPoll::Pending => return Ok(false),
        AgentPoll::Record(record) => {
            state.observe_record(record).map_err(LoopError::State)?;
        },
        AgentPoll::RequestTrace(entry) => {
            state.observe_request_trace(entry);
        },
        AgentPoll::Durability(durability) => {
            state
                .observe_durability(durability)
                .map_err(LoopError::State)?;
        },
        AgentPoll::Submission(outcome) => {
            state
                .observe_submission_outcome(outcome)
                .map_err(LoopError::State)?;
        },
        AgentPoll::Closed => {
            return Err(LoopError::Agent(
                "the agent connection closed unexpectedly".to_owned(),
            ));
        },
    }
    Ok(true)
}

pub(in crate::runner) fn apply_workspace_poll(
    state: &mut TuiState,
    connection: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    observation: Result<WorkspaceReferencePoll, String>,
) -> bool {
    match observation {
        Ok(WorkspaceReferencePoll::Pending) => false,
        Ok(WorkspaceReferencePoll::Update(update)) => matches!(
            state.observe_workspace_reference_update(update),
            StateEffect::Redraw
        ),
        Err(error) => {
            let changed = matches!(
                state.observe_workspace_reference_failure(error),
                StateEffect::Redraw
            );
            *connection = None;
            changed
        },
    }
}

pub(in crate::runner) fn apply_skill_poll(
    state: &mut TuiState,
    connection: &mut Option<Box<dyn SkillReferenceConnection>>,
    observation: Result<SkillReferencePoll, String>,
) -> bool {
    match observation {
        Ok(SkillReferencePoll::Pending) => false,
        Ok(SkillReferencePoll::Update(update)) => matches!(
            state.observe_skill_reference_update(update),
            StateEffect::Redraw
        ),
        Err(error) => {
            let changed = matches!(
                state.observe_skill_reference_failure(error),
                StateEffect::Redraw
            );
            *connection = None;
            changed
        },
    }
}

pub(super) fn dispatch_workspace_search(
    connection: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    state: &mut TuiState,
    request: yo_core::WorkspaceReferenceSearchRequest,
) {
    let result = connection
        .as_deref_mut()
        .ok_or_else(|| "workspace search is unavailable".to_owned())
        .and_then(|connection| connection.search(request));
    if let Err(error) = result {
        state.observe_workspace_reference_failure(error);
    }
}

pub(super) fn dispatch_skill_search(
    connection: &mut Option<Box<dyn SkillReferenceConnection>>,
    state: &mut TuiState,
    request: yo_core::SkillReferenceSearchRequest,
) {
    let result = connection
        .as_deref_mut()
        .ok_or_else(|| "skill search is unavailable".to_owned())
        .and_then(|connection| connection.search(request));
    if let Err(error) = result {
        state.observe_skill_reference_failure(error);
    }
}
