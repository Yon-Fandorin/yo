use serde_json::json;
use yo_backend::{BackendAdapter, transport::JsonMessagePeer};
use yo_core::{
    AgentCommand, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity, BackendPoll,
    BackendRequestEvidence, BackendResumeTarget, BackendStopHandle, ContinuationStrategy, HostId,
    ImageInputCapability, InputImageHistory, SessionId,
};

use super::{
    events,
    input::project_input,
    state::{Backend, SessionBinding, WireTurnBinding},
};
use crate::{
    BACKEND_KIND, MODEL_IDENTITY_SCHEMA,
    binding::{binding_account, decode_optional_account, model_and_provider},
    protocol,
};

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => {
                self.validate_direct_input(&input)?;
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let projected_input = project_input(&input)?;
                let mut params = json!({
                    "threadId": thread_id,
                    "input": projected_input,
                    "cwd": self.cwd,
                });
                self.apply_turn_policy(&mut params);
                let call = self.client.call("turn/start", params)?;
                let wire_turn = protocol::string_at(&call.result, &["turn", "id"])?.to_owned();
                self.turns.insert(turn, wire_turn.clone());
                self.wire_turns.insert(
                    wire_turn.clone(),
                    WireTurnBinding {
                        turn,
                        interrupted: false,
                        finished: false,
                    },
                );
                if !input.images().is_empty() {
                    self.input_image_history = InputImageHistory::ContainsImages;
                }
                Ok(BackendCommandEvidence::RequestAccepted(
                    BackendRequestEvidence::new(
                        "codex.app-server/turn-start/v1",
                        json_rpc_identity(call.request_id),
                        accepted_request_identity(call.request_id, &wire_turn),
                    ),
                ))
            },
            AgentCommand::SteerTurn { turn, input } => {
                self.validate_direct_input(&input)?;
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                let projected_input = project_input(&input)?;
                let call = self.client.call(
                    "turn/steer",
                    json!({
                        "threadId": thread_id,
                        "expectedTurnId": &turn_id,
                        "input": projected_input,
                    }),
                )?;
                let accepted = protocol::string_at(&call.result, &["turnId"])?;
                if accepted != turn_id {
                    return Err(protocol::protocol_failure(format!(
                        "Codex steer accepted Turn `{accepted}` instead of `{turn_id}`"
                    )));
                }
                if !input.images().is_empty() {
                    self.input_image_history = InputImageHistory::ContainsImages;
                }
                Ok(BackendCommandEvidence::RequestAccepted(
                    BackendRequestEvidence::new(
                        "codex.app-server/turn-steer/v1",
                        json_rpc_identity(call.request_id),
                        accepted_request_identity(call.request_id, accepted),
                    ),
                ))
            },
            AgentCommand::InterruptTurn { turn } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                self.client.call(
                    "turn/interrupt",
                    json!({ "threadId": thread_id, "turnId": turn_id }),
                )?;
                Ok(BackendCommandEvidence::None)
            },
            AgentCommand::RespondToActivity { request, response } => {
                self.respond_to_activity(request, response)
            },
            AgentCommand::CompactContext { .. } => Err(BackendFailure::new(
                BackendFailureKind::CommandRejected,
                "Codex delegated Sessions do not use Yo-managed context compaction",
            )),
        }
    }

    pub(super) fn create_session(
        &mut self,
        session_id: SessionId,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.initialize()?;
        let mut params = json!({
            "cwd": self.cwd,
            "serviceName": "yo",
        });
        if let Some((account, model)) = &self.new_session_target {
            if self.model_rebind_target.is_some() || self.account.as_ref() != Some(account) {
                return Err(BackendFailure::new(
                    BackendFailureKind::Session,
                    "new Session requires the selected authenticated account and no rebind target",
                ));
            }
            params["model"] = json!(model.as_str());
        }
        self.apply_thread_policy(&mut params);
        if let Some(secret_tool) = self.secret_tool
            && !self.read_only_review
        {
            let wire_supported = super::secret_probe::wire_version_supported(
                self.backend_version.as_deref().unwrap_or_default(),
            );
            if !wire_supported && secret_tool == super::state::DelegatedSecretTool::Probe {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "the secret-entry probe requires the reviewed Codex 0.155.1 dynamic-tool wire",
                ));
            }
            if wire_supported {
                params["dynamicTools"] = json!([super::secret_probe::tool_spec(secret_tool)]);
            }
        }
        let result = self.client.call("thread/start", params)?.result;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?.to_owned();
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        if self
            .new_session_target
            .as_ref()
            .is_some_and(|(_, expected)| model != expected.as_str())
        {
            return Err(protocol::protocol_failure(
                "new Session returned a different model",
            ));
        }
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        self.selected_model = Some(model.to_owned());
        let backend_version = self.backend_version.clone().ok_or_else(|| {
            protocol::protocol_failure("Codex backend version was not retained after initialize")
        })?;
        let model_value = json!({
            "model": model,
            "provider": model_provider,
        })
        .to_string();
        self.client.bind_notice_thread(&thread_id);
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: thread_id.clone(),
        });
        if self.image_wire_supported {
            self.refresh_model_capability(model);
        } else {
            self.image_capability = ImageInputCapability::Unknown;
        }
        Ok(BackendCommandEvidence::BindingOpened(
            BackendBindingEvidence::new(
                BACKEND_KIND,
                backend_version,
                self.binding_identity(backend_session_id, &thread_id)?,
                BackendIdentity::new(MODEL_IDENTITY_SCHEMA, model_value),
                BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
                ContinuationStrategy::BackendManagedState,
            ),
        ))
    }

    pub(super) fn initialize(&mut self) -> Result<(), BackendFailure> {
        if !self.initialized {
            let initialize = self.client.initialize()?;
            let account_result = self
                .client
                .call("account/read", json!({ "refreshToken": false }))?
                .result;
            self.backend_version = Some(initialize.user_agent);
            self.image_wire_supported = protocol::image_wire_version_supported(
                self.backend_version.as_deref().unwrap_or_default(),
            );
            self.image_capability = ImageInputCapability::Unknown;
            self.account = decode_optional_account(&HostId::codex(), &account_result)?;
            self.initialized = true;
        }
        Ok(())
    }

    pub(super) fn verify(&mut self) -> Result<(), BackendFailure> {
        self.initialize()
    }

    pub(super) fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if self.new_session_target.is_some() {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "a new-session target cannot be used to resume or fork a Session",
            ));
        }
        self.resume_binding_with_history(
            target.session_id(),
            target.binding(),
            target.input_image_history(),
        )
    }

    #[cfg(test)]
    pub(super) fn resume_binding(
        &mut self,
        session_id: SessionId,
        binding: &BackendBindingEvidence,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_binding_with_history(session_id, binding, InputImageHistory::TextOnly)
    }

    pub(super) fn resume_binding_with_history(
        &mut self,
        session_id: SessionId,
        binding: &BackendBindingEvidence,
        image_history: InputImageHistory,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if binding.backend_kind() != BACKEND_KIND {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!(
                    "Codex cannot resume backend kind `{}`",
                    binding.backend_kind()
                ),
            ));
        }
        let locator = binding.session_locator();
        if locator.schema() != "codex.app-server/thread-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Codex Session locator `{}`", locator.schema()),
            ));
        }
        let thread_id = locator.value();
        self.initialize()?;
        self.validate_execution_binding(binding)?;
        let (expected_model, _) = model_and_provider(binding.model_identity())?;
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(expected_model.as_str())?;
        }
        let mut params = json!({ "threadId": thread_id });
        if image_history != InputImageHistory::TextOnly {
            params["model"] = json!(expected_model.as_str());
        }
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/resume", params)?.result;
        let resumed_thread = protocol::string_at(&result, &["thread", "id"])?;
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        let binding_identity =
            self.binding_identity_for_resume(binding, backend_session_id, resumed_thread)?;
        let model_identity = BackendIdentity::new(
            MODEL_IDENTITY_SCHEMA,
            json!({ "model": model, "provider": model_provider }).to_string(),
        );
        let evidence = BackendBindingEvidence::new(
            BACKEND_KIND,
            self.backend_version.clone().ok_or_else(|| {
                protocol::protocol_failure(
                    "Codex backend version was not retained after resume initialize",
                )
            })?,
            binding_identity,
            model_identity,
            BackendIdentity::new("codex.app-server/thread-locator/v1", resumed_thread),
            ContinuationStrategy::BackendManagedState,
        );
        if !binding.same_resume_identity(&evidence) {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex resumed a binding whose thread, Session, model, or provider identity differs from the durable Continuation Anchor",
            ));
        }
        self.selected_model = Some(model.to_owned());
        if self.image_wire_supported {
            self.refresh_model_capability(model);
        } else {
            self.image_capability = ImageInputCapability::Unknown;
        }
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(model)?;
        }
        self.client.bind_notice_thread(resumed_thread);
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: resumed_thread.to_owned(),
        });
        self.input_image_history = image_history;
        Ok(evidence)
    }

    pub(super) fn resume_session_rebinding_model(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.rebind_model_with_history(
            target.session_id(),
            target.binding(),
            target.input_image_history(),
        )
    }

    #[cfg(test)]
    pub(super) fn rebind_model(
        &mut self,
        session_id: SessionId,
        source: &BackendBindingEvidence,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.rebind_model_with_history(session_id, source, InputImageHistory::TextOnly)
    }

    pub(super) fn rebind_model_with_history(
        &mut self,
        session_id: SessionId,
        source: &BackendBindingEvidence,
        image_history: InputImageHistory,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if source.backend_kind() != BACKEND_KIND
            || source.continuation_strategy() != ContinuationStrategy::BackendManagedState
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex native model rebind requires a Codex backend-managed source binding",
            ));
        }
        if source.session_locator().schema() != "codex.app-server/thread-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex native model rebind requires the supported thread locator",
            ));
        }
        let (requested_account, requested_model) =
            self.model_rebind_target.clone().ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "Codex native model rebind has no exact account and model target",
                )
            })?;
        self.initialize()?;
        self.validate_execution_binding(source)?;
        let source_account = binding_account(source.binding_identity())?.ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "this legacy Codex binding predates verified account identity and cannot be rebound",
                )
            })?;
        if source_account != requested_account || self.account.as_ref() != Some(&requested_account)
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex native model rebind account differs from the source Session account",
            ));
        }
        let (source_model, source_provider) = model_and_provider(source.model_identity())?;
        if source_model == requested_model {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex native model rebind target is already active",
            ));
        }
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(requested_model.as_str())?;
        }

        let source_thread = source.session_locator().value();
        let mut params = json!({
            "threadId": source_thread,
            "model": requested_model.as_str(),
            "cwd": self.cwd,
        });
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/fork", params)?.result;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?;
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        if thread_id == source_thread
            || model != requested_model.as_str()
            || model_provider != source_provider
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex thread/fork did not return a distinct thread with the exact requested model and source provider",
            ));
        }
        let evidence = BackendBindingEvidence::new(
            BACKEND_KIND,
            self.backend_version.clone().ok_or_else(|| {
                protocol::protocol_failure(
                    "Codex backend version was not retained after rebind initialize",
                )
            })?,
            self.binding_identity(backend_session_id, thread_id)?,
            BackendIdentity::new(
                MODEL_IDENTITY_SCHEMA,
                json!({ "model": model, "provider": model_provider }).to_string(),
            ),
            BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
            ContinuationStrategy::BackendManagedState,
        );
        self.selected_model = Some(model.to_owned());
        if self.image_wire_supported {
            self.refresh_model_capability(model);
        } else {
            self.image_capability = ImageInputCapability::Unknown;
        }
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(model)?;
        }
        self.client.bind_notice_thread(thread_id);
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: thread_id.to_owned(),
        });
        self.input_image_history = image_history;
        Ok(evidence)
    }

    pub(super) fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(BackendPoll::Event(event));
        }
        if let Some(terminal) = &self.terminal_poll {
            return terminal.clone().map(|()| BackendPoll::Closed);
        }
        events::poll_client_message(self)
    }

    pub(super) fn shutdown(&mut self) -> Result<(), BackendFailure> {
        for binding in self.requests.values_mut() {
            if let super::state::RequestKind::Input(questions) = &mut binding.kind {
                questions.discard_secret_values(false);
            }
        }
        self.client.shutdown()
    }
}

impl<P: JsonMessagePeer> BackendAdapter for Backend<P> {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        self.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_session(target)
    }

    fn resume_session_rebinding_model(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_session_rebinding_model(target)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.shutdown()
    }
}

fn json_rpc_identity(request_id: u64) -> BackendIdentity {
    BackendIdentity::new(
        "codex.app-server/json-rpc-request/v1",
        request_id.to_string(),
    )
}

fn accepted_request_identity(request_id: u64, turn_id: &str) -> BackendIdentity {
    BackendIdentity::new(
        "codex.app-server/accepted-request/v1",
        json!({ "jsonRpcId": request_id, "turnId": turn_id }).to_string(),
    )
}
