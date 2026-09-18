//! 커넥터 폴링과 응답 이벤트 전달.

use std::sync::atomic::Ordering;

use serde_json::json;
use yo_core::{
    ActivityKind, BackendEvent, BackendFailure, BackendFailureKind, BackendPoll,
    ModelConnectorEvent, ModelConnectorPoll,
};

use super::super::{
    CompactionState, IdleCompactionState, NativeModelBackend, failure, map_connector_turn,
};

pub(super) fn poll_event(backend: &mut NativeModelBackend) -> Result<BackendPoll, BackendFailure> {
    if let Some(event) = backend.pop_event() {
        return Ok(BackendPoll::Event(event));
    }
    if backend.closed {
        return Ok(BackendPoll::Closed);
    }
    if matches!(
        backend.idle_compaction,
        Some(IdleCompactionState::AwaitingCheckpoint { .. })
    ) {
        let Some(IdleCompactionState::AwaitingCheckpoint { replay }) =
            backend.idle_compaction.take()
        else {
            unreachable!("checkpoint-ready idle compaction was checked")
        };
        backend.replay = replay;
        backend.replay_groups = vec![backend.replay.items().to_vec()];
        return Ok(BackendPoll::Pending);
    }
    if matches!(
        backend.idle_compaction,
        Some(IdleCompactionState::Summarizing { .. })
    ) {
        let poll = {
            let Some(IdleCompactionState::Summarizing { stream, .. }) =
                backend.idle_compaction.as_mut()
            else {
                unreachable!("active idle summary was checked")
            };
            stream.poll()
        };
        match poll {
            Ok(ModelConnectorPoll::Event(event)) => {
                if let Err(error) = backend.apply_idle_compaction_event(event) {
                    if error.kind() == BackendFailureKind::CommandRejected {
                        backend.cleanup_idle_compaction();
                        return Err(error);
                    }
                    backend.context_exhausted = true;
                    backend.cleanup_idle_compaction();
                    return Err(if error.kind() == BackendFailureKind::ContextExhausted {
                        error
                    } else {
                        failure(
                            BackendFailureKind::ContextExhausted,
                            format!("context_exhausted: idle context summary failed: {error}"),
                        )
                    });
                }
            },
            Ok(ModelConnectorPoll::Pending) => {},
            Ok(ModelConnectorPoll::Closed) => {
                backend.context_exhausted = true;
                backend.cleanup_idle_compaction();
                return Err(failure(
                    BackendFailureKind::ContextExhausted,
                    "context_exhausted: idle context summary stream closed without a terminal event",
                ));
            },
            Err(error) => {
                backend.context_exhausted = true;
                backend.cleanup_idle_compaction();
                return Err(failure(
                    BackendFailureKind::ContextExhausted,
                    format!("context_exhausted: idle context summary request failed: {error}"),
                ));
            },
        }
        return Ok(backend
            .pop_event()
            .map_or(BackendPoll::Pending, BackendPoll::Event));
    }
    if backend.shared_stop.requested.swap(false, Ordering::AcqRel)
        && let Some(turn) = backend.turn.as_ref().map(|state| state.turn)
    {
        backend.interrupt(turn)?;
        return Ok(BackendPoll::Event(
            backend.pop_event().expect("interrupt queues an event"),
        ));
    }
    if backend.turn.as_ref().is_some_and(|state| {
        matches!(
            state.compaction,
            Some(CompactionState::AwaitingCheckpoint { .. })
        )
    }) {
        let mut state = backend
            .turn
            .take()
            .expect("checkpoint-ready Turn was checked");
        let Some(CompactionState::AwaitingCheckpoint { replay }) = state.compaction.take() else {
            unreachable!("checkpoint-ready compaction state was checked")
        };
        backend.replay = replay;
        backend.replay_groups = vec![backend.replay.items().to_vec()];
        state.delta.clear();
        state.compaction_attempted = true;
        if let Err(error) = backend.start_model_round(&mut state) {
            backend.fail_or_exhaust_turn(&mut state, error);
        } else {
            backend
                .events
                .push_back(BackendEvent::ModelRequestAccepted {
                    turn: state.turn,
                    evidence: backend.request_evidence(state.turn),
                });
            backend.turn = Some(state);
        }
    } else if backend.turn.as_ref().is_some_and(|state| {
        state
            .prepared_secret_request
            .as_ref()
            .is_some_and(|prepared| prepared.armed && prepared.request.is_some())
    }) {
        let mut state = backend
            .turn
            .take()
            .expect("prepared secret Turn was checked");
        if let Err(_error) = backend.start_prepared_secret_request(&mut state) {
            state.prepared_secret_request = None;
            backend.fail_turn(
                &mut state,
                "terminal secret request failed after submission; delivery outcome is unknown"
                    .to_owned(),
            );
        } else {
            backend
                .events
                .push_back(BackendEvent::ModelRequestAccepted {
                    turn: state.turn,
                    evidence: backend.request_evidence(state.turn),
                });
            backend.turn = Some(state);
        }
    } else if backend
        .turn
        .as_ref()
        .is_some_and(|state| state.active_tool.is_some())
    {
        backend.poll_tool()?;
    } else if backend
        .turn
        .as_ref()
        .is_some_and(|state| state.dispatch_tool.is_some())
    {
        let mut state = backend.turn.take().expect("active Turn was checked");
        let (call, activity) = state
            .dispatch_tool
            .take()
            .expect("dispatch-ready tool was checked");
        if let Err(error) = backend.start_tool_execution(&mut state, call, activity) {
            backend.fail_or_exhaust_turn(&mut state, error);
        } else {
            backend.turn = Some(state);
        }
    } else if backend
        .turn
        .as_ref()
        .is_some_and(|state| state.ready_tool.is_some())
    {
        let mut state = backend.turn.take().expect("active Turn was checked");
        let call = state.ready_tool.take().expect("ready tool was checked");
        match backend.next_activity(state.turn) {
            Ok(activity) => {
                backend.queue_activity_text(
                    activity,
                    ActivityKind::ToolResult,
                    json!({
                        "call_id": call.call_id(),
                        "tool_id": call.definition().id().as_str(),
                        "execution_host": backend.tool_host.identity(),
                        "attempt": 1,
                    })
                    .to_string(),
                    None,
                );
                state.dispatch_tool = Some((call, activity));
                backend.turn = Some(state);
            },
            Err(error) => backend.fail_turn(&mut state, error.to_string()),
        }
    } else if backend
        .turn
        .as_ref()
        .is_some_and(|state| state.start_next_round)
    {
        let mut state = backend.turn.take().expect("active Turn was checked");
        state.start_next_round = false;
        if let Err(error) = backend.start_model_round(&mut state) {
            backend.fail_or_exhaust_turn(&mut state, error);
        } else {
            if state.compaction.is_none() && state.stream.is_some() {
                backend
                    .events
                    .push_back(BackendEvent::ModelRequestAccepted {
                        turn: state.turn,
                        evidence: backend.request_evidence(state.turn),
                    });
            }
            backend.turn = Some(state);
        }
    } else if backend
        .turn
        .as_ref()
        .is_some_and(|state| state.stream.is_some())
    {
        let poll = {
            let state = backend.turn.as_mut().expect("active Turn was checked");
            state
                .stream
                .as_mut()
                .expect("response stream was checked")
                .poll()
        };
        match poll {
            Err(error) => {
                let mut state = backend.turn.take().expect("active Turn was checked");
                backend.observe_connector_failure(state.turn, &error);
                if state.terminal_secret_request {
                    state.prepared_secret_request = None;
                    backend.fail_turn(
                        &mut state,
                        "terminal secret request failed after submission; delivery outcome is unknown"
                            .to_owned(),
                    );
                } else if matches!(state.compaction, Some(CompactionState::Summarizing { .. })) {
                    backend.context_exhausted = true;
                    backend.exhaust_turn(
                        &mut state,
                        format!(
                            "context_exhausted: context summary request failed: {}",
                            map_connector_turn(error)
                        ),
                    );
                } else {
                    backend.fail_turn(&mut state, map_connector_turn(error).to_string());
                }
            },
            Ok(ModelConnectorPoll::Event(event)) => handle_response_event(backend, event)?,
            Ok(ModelConnectorPoll::Closed) => {
                let mut state = backend.turn.take().expect("active Turn was checked");
                backend.observe_model_request(
                    state.turn,
                    yo_core::ModelRequestOutcome::Failed(
                        yo_core::ModelRequestFailureKind::Protocol,
                    ),
                );
                if state.terminal_secret_request {
                    state.prepared_secret_request = None;
                    backend.fail_turn(
                        &mut state,
                        "terminal secret request failed after submission; delivery outcome is unknown"
                            .to_owned(),
                    );
                } else if matches!(state.compaction, Some(CompactionState::Summarizing { .. })) {
                    backend.context_exhausted = true;
                    backend.exhaust_turn(
                        &mut state,
                        "context_exhausted: context summary stream closed without a terminal event"
                            .to_owned(),
                    );
                } else {
                    backend.fail_turn(
                        &mut state,
                        "model connector stream closed without a terminal event".to_owned(),
                    );
                }
            },
            Ok(ModelConnectorPoll::Pending) => {},
        }
    }
    Ok(backend
        .pop_event()
        .map_or(BackendPoll::Pending, BackendPoll::Event))
}

fn handle_response_event(
    backend: &mut NativeModelBackend,
    event: ModelConnectorEvent,
) -> Result<(), BackendFailure> {
    let mut state = backend.turn.take().ok_or_else(|| {
        failure(
            BackendFailureKind::Protocol,
            "response event has no active Turn",
        )
    })?;
    let was_compacting = matches!(state.compaction, Some(CompactionState::Summarizing { .. }));
    let result = if was_compacting {
        backend.apply_compaction_response_event(&mut state, event)
    } else {
        backend.apply_response_event(&mut state, event)
    };
    if let Err(error) = result {
        if was_compacting || error.kind() == BackendFailureKind::ContextExhausted {
            backend.observe_model_request(
                state.turn,
                yo_core::ModelRequestOutcome::Failed(
                    yo_core::ModelRequestFailureKind::ResponseLimit,
                ),
            );
            backend.context_exhausted = true;
            backend.exhaust_turn(
                &mut state,
                if was_compacting && error.kind() != BackendFailureKind::ContextExhausted {
                    format!("context_exhausted: context summary failed: {error}")
                } else {
                    error.to_string()
                },
            );
        } else {
            if error.kind() == BackendFailureKind::Protocol {
                backend.observe_model_request(
                    state.turn,
                    yo_core::ModelRequestOutcome::Failed(
                        yo_core::ModelRequestFailureKind::Protocol,
                    ),
                );
            }
            backend.fail_turn(&mut state, error.to_string());
        }
    }
    if backend.turn.is_none()
        && !backend.events.iter().any(|event| {
            matches!(
                event,
                BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
            )
        })
    {
        backend.turn = Some(state);
    }
    Ok(())
}
