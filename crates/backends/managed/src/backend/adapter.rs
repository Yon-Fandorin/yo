//! Backend lifecycle scheduler and ordinary-versus-compaction response dispatch.

use std::sync::{Arc, atomic::Ordering};

use serde_json::json;
use yo_backend::BackendAdapter;
use yo_core::{
    ActivityKind, ActivityResponse, AgentCommand, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendPoll,
    BackendResumeTarget, BackendStopHandle, ModelConnectorEvent, ModelConnectorPoll,
};

use super::{
    CompactionState, IdleCompactionState, NativeModelBackend, failure,
    identity::semantically_equal_native_binding_identity, map_connector_cleanup,
    map_connector_turn, map_tool_cleanup,
};

impl BackendAdapter for NativeModelBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        let shared = Arc::clone(&self.shared_stop);
        BackendStopHandle::new(move || {
            shared.requested.store(true, Ordering::Release);
            if let Ok(guard) = shared.response.lock()
                && let Some(cancellation) = guard.as_ref()
            {
                cancellation.cancel();
            }
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if self.closed || self.session.is_some() || self.turn.is_some() {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend is not available for resume",
            ));
        }
        let expected = self.binding_evidence(target.session_id());
        if !same_native_resume_identity(&expected, target.binding())
            || target.model_replay().contract() != Some(&self.contract)
        {
            return Err(failure(
                BackendFailureKind::Session,
                "durable native model binding or replay contract does not match current configuration",
            ));
        }
        self.session = Some(target.session_id());
        self.replay = target.model_replay().clone();
        self.restore_context_state(target)?;
        Ok(target.binding().clone())
    }

    fn resume_session_replacing_binding(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if self.closed || self.session.is_some() || self.turn.is_some() {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend is not available for binding replacement",
            ));
        }
        if target.model_replay().contract() != Some(&self.contract) {
            return Err(failure(
                BackendFailureKind::Session,
                "durable exact replay contract does not match the replacement binding",
            ));
        }
        self.session = Some(target.session_id());
        self.replay = target.model_replay().clone();
        self.restore_context_state(target)?;
        Ok(self.binding_evidence(target.session_id()))
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.closed {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend is closed",
            ));
        }
        match command {
            AgentCommand::CreateSession { session_id } => {
                if self.session.is_some() {
                    return Err(failure(
                        BackendFailureKind::Session,
                        "native backend already has a Session",
                    ));
                }
                self.session = Some(session_id);
                self.context_policy_active = true;
                self.events.push_back(BackendEvent::ContextPolicyChanged {
                    policy: self.config.context_policy.clone(),
                });
                Ok(BackendCommandEvidence::BindingOpened(
                    self.binding_evidence(session_id),
                ))
            },
            AgentCommand::StartTurn { turn, input } => self.start_turn(turn, input.into_string()),
            AgentCommand::SteerTurn { .. } => Err(failure(
                BackendFailureKind::Unsupported,
                "native model loop does not support steering",
            )),
            AgentCommand::InterruptTurn { turn } => self.interrupt(turn),
            AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(decision),
            } => self.respond_to_approval(request, decision),
            AgentCommand::RespondToActivity { .. } => Err(failure(
                BackendFailureKind::Unsupported,
                "native model loop only accepts approval responses",
            )),
            AgentCommand::CompactContext { guidance } => {
                self.start_idle_compaction(guidance)?;
                Ok(BackendCommandEvidence::None)
            },
        }
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if let Some(event) = self.pop_event() {
            return Ok(BackendPoll::Event(event));
        }
        if self.closed {
            return Ok(BackendPoll::Closed);
        }
        if matches!(
            self.idle_compaction,
            Some(IdleCompactionState::AwaitingCheckpoint { .. })
        ) {
            let Some(IdleCompactionState::AwaitingCheckpoint { replay }) =
                self.idle_compaction.take()
            else {
                unreachable!("checkpoint-ready idle compaction was checked")
            };
            self.replay = replay;
            self.replay_groups = vec![self.replay.items().to_vec()];
            return Ok(BackendPoll::Pending);
        }
        if matches!(
            self.idle_compaction,
            Some(IdleCompactionState::Summarizing { .. })
        ) {
            let poll = {
                let Some(IdleCompactionState::Summarizing { stream, .. }) =
                    self.idle_compaction.as_mut()
                else {
                    unreachable!("active idle summary was checked")
                };
                stream.poll()
            };
            match poll {
                Ok(ModelConnectorPoll::Event(event)) => {
                    if let Err(error) = self.apply_idle_compaction_event(event) {
                        self.context_exhausted = true;
                        self.cleanup_idle_compaction();
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
                    self.context_exhausted = true;
                    self.cleanup_idle_compaction();
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: idle context summary stream closed without a terminal event",
                    ));
                },
                Err(error) => {
                    self.context_exhausted = true;
                    self.cleanup_idle_compaction();
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        format!("context_exhausted: idle context summary request failed: {error}"),
                    ));
                },
            }
            return Ok(self
                .pop_event()
                .map_or(BackendPoll::Pending, BackendPoll::Event));
        }
        if self.shared_stop.requested.swap(false, Ordering::AcqRel)
            && let Some(turn) = self.turn.as_ref().map(|state| state.turn)
        {
            self.interrupt(turn)?;
            return Ok(BackendPoll::Event(
                self.pop_event().expect("interrupt queues an event"),
            ));
        }
        if self.turn.as_ref().is_some_and(|state| {
            matches!(
                state.compaction,
                Some(CompactionState::AwaitingCheckpoint { .. })
            )
        }) {
            let mut state = self.turn.take().expect("checkpoint-ready Turn was checked");
            let Some(CompactionState::AwaitingCheckpoint { replay }) = state.compaction.take()
            else {
                unreachable!("checkpoint-ready compaction state was checked")
            };
            self.replay = replay;
            self.replay_groups = vec![self.replay.items().to_vec()];
            state.delta.clear();
            state.compaction_attempted = true;
            if let Err(error) = self.start_model_round(&mut state) {
                self.fail_or_exhaust_turn(&mut state, error);
            } else {
                self.events.push_back(BackendEvent::ModelRequestAccepted {
                    turn: state.turn,
                    evidence: self.request_evidence(state.turn),
                });
                self.turn = Some(state);
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.active_tool.is_some())
        {
            self.poll_tool()?;
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.dispatch_tool.is_some())
        {
            let mut state = self.turn.take().expect("active Turn was checked");
            let (call, activity) = state
                .dispatch_tool
                .take()
                .expect("dispatch-ready tool was checked");
            if let Err(error) = self.start_tool_execution(&mut state, call, activity) {
                self.fail_or_exhaust_turn(&mut state, error);
            } else {
                self.turn = Some(state);
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.ready_tool.is_some())
        {
            let mut state = self.turn.take().expect("active Turn was checked");
            let call = state.ready_tool.take().expect("ready tool was checked");
            match self.next_activity(state.turn) {
                Ok(activity) => {
                    self.queue_activity_text(
                        activity,
                        ActivityKind::ToolResult,
                        json!({
                            "call_id": call.call_id(),
                            "tool_id": call.definition().id().as_str(),
                            "execution_host": self.tool_host.identity(),
                            "attempt": 1,
                        })
                        .to_string(),
                        None,
                    );
                    state.dispatch_tool = Some((call, activity));
                    self.turn = Some(state);
                },
                Err(error) => self.fail_turn(&mut state, error.to_string()),
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.start_next_round)
        {
            let mut state = self.turn.take().expect("active Turn was checked");
            state.start_next_round = false;
            if let Err(error) = self.start_model_round(&mut state) {
                self.fail_or_exhaust_turn(&mut state, error);
            } else {
                if state.compaction.is_none() && state.stream.is_some() {
                    self.events.push_back(BackendEvent::ModelRequestAccepted {
                        turn: state.turn,
                        evidence: self.request_evidence(state.turn),
                    });
                }
                self.turn = Some(state);
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.stream.is_some())
        {
            let poll = {
                let state = self.turn.as_mut().expect("active Turn was checked");
                state
                    .stream
                    .as_mut()
                    .expect("response stream was checked")
                    .poll()
            };
            match poll {
                Err(error) => {
                    let mut state = self.turn.take().expect("active Turn was checked");
                    self.observe_connector_failure(state.turn, &error);
                    if matches!(state.compaction, Some(CompactionState::Summarizing { .. })) {
                        self.context_exhausted = true;
                        self.exhaust_turn(
                            &mut state,
                            format!(
                                "context_exhausted: context summary request failed: {}",
                                map_connector_turn(error)
                            ),
                        );
                    } else {
                        self.fail_turn(&mut state, map_connector_turn(error).to_string());
                    }
                },
                Ok(ModelConnectorPoll::Event(event)) => self.handle_response_event(event)?,
                Ok(ModelConnectorPoll::Closed) => {
                    let mut state = self.turn.take().expect("active Turn was checked");
                    self.observe_model_request(
                        state.turn,
                        yo_core::ModelRequestOutcome::Failed(
                            yo_core::ModelRequestFailureKind::Protocol,
                        ),
                    );
                    if matches!(state.compaction, Some(CompactionState::Summarizing { .. })) {
                        self.context_exhausted = true;
                        self.exhaust_turn(
                            &mut state,
                            "context_exhausted: context summary stream closed without a terminal event"
                                .to_owned(),
                        );
                    } else {
                        self.fail_turn(
                            &mut state,
                            "model connector stream closed without a terminal event".to_owned(),
                        );
                    }
                },
                Ok(ModelConnectorPoll::Pending) => {},
            }
        }
        Ok(self
            .pop_event()
            .map_or(BackendPoll::Pending, BackendPoll::Event))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        if let Some(result) = &self.shutdown_result {
            return result.clone();
        }
        let mut result = Ok(());
        if let Some(mut state) = self.turn.take() {
            if let Some(mut stream) = state.stream.take() {
                stream.cancel();
                if let Err(error) = stream.shutdown() {
                    result = Err(map_connector_cleanup(error));
                }
            }
            if let Some(mut active) = state.active_tool.take() {
                active.execution.cancel();
                if let Err(error) = active.execution.shutdown()
                    && result.is_ok()
                {
                    result = Err(map_tool_cleanup(error));
                }
            }
        }
        if let Some(IdleCompactionState::Summarizing { mut stream, .. }) =
            self.idle_compaction.take()
        {
            stream.cancel();
            if let Err(error) = stream.shutdown()
                && result.is_ok()
            {
                result = Err(map_connector_cleanup(error));
            }
        }
        if let Err(error) = self.tool_host.shutdown()
            && result.is_ok()
        {
            result = Err(map_tool_cleanup(error));
        }
        self.events.clear();
        self.open_activities.clear();
        self.closed = true;
        self.shutdown_result = Some(result.clone());
        result
    }
}

impl NativeModelBackend {
    fn handle_response_event(&mut self, event: ModelConnectorEvent) -> Result<(), BackendFailure> {
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "response event has no active Turn",
            )
        })?;
        let was_compacting = matches!(state.compaction, Some(CompactionState::Summarizing { .. }));
        let result = if was_compacting {
            self.apply_compaction_response_event(&mut state, event)
        } else {
            self.apply_response_event(&mut state, event)
        };
        if let Err(error) = result {
            if was_compacting || error.kind() == BackendFailureKind::ContextExhausted {
                self.observe_model_request(
                    state.turn,
                    yo_core::ModelRequestOutcome::Failed(
                        yo_core::ModelRequestFailureKind::ResponseLimit,
                    ),
                );
                self.context_exhausted = true;
                self.exhaust_turn(
                    &mut state,
                    if was_compacting && error.kind() != BackendFailureKind::ContextExhausted {
                        format!("context_exhausted: context summary failed: {error}")
                    } else {
                        error.to_string()
                    },
                );
            } else {
                if error.kind() == BackendFailureKind::Protocol {
                    self.observe_model_request(
                        state.turn,
                        yo_core::ModelRequestOutcome::Failed(
                            yo_core::ModelRequestFailureKind::Protocol,
                        ),
                    );
                }
                self.fail_turn(&mut state, error.to_string());
            }
        }
        if self.turn.is_none()
            && !self.events.iter().any(|event| {
                matches!(
                    event,
                    BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
                )
            })
        {
            self.turn = Some(state);
        }
        Ok(())
    }
}

fn same_native_resume_identity(
    current: &BackendBindingEvidence,
    durable: &BackendBindingEvidence,
) -> bool {
    current.backend_kind() == durable.backend_kind()
        && current.model_identity() == durable.model_identity()
        && current.session_locator() == durable.session_locator()
        && current.continuation_strategy() == durable.continuation_strategy()
        && semantically_equal_native_binding_identity(
            current.binding_identity(),
            durable.binding_identity(),
        )
}
