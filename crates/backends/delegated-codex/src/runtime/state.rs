use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    num::NonZeroU64,
    sync::Arc,
};

use serde_json::{Map, Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    AccountId, ActivityApproval, ActivityId, ActivityRef, ActivityRequestRef,
    BackendBindingEvidence, BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind,
    BackendIdentity, ImageInputCapability, InputImageHistory, ModelId, QuestionChoice, RequestId,
    SessionId, TurnRef,
    interview::{Answer, AnswerResponse, Capture},
};

use crate::{
    LEGACY_READ_ONLY_BINDING_SCHEMA, LEGACY_STANDARD_BINDING_SCHEMA, READ_ONLY_BINDING_SCHEMA,
    READ_ONLY_REVIEW_PROFILE, STANDARD_BINDING_SCHEMA, binding::binding_account,
    client::AppServerClient, protocol,
};

pub(super) struct SessionBinding {
    pub(super) yo: SessionId,
    pub(super) codex: String,
}

pub(super) struct ItemBinding {
    pub(super) activity: ActivityRef,
    pub(super) dynamic_tool_call: Option<DynamicToolCall>,
    pub(super) public_summary: Option<BTreeMap<u64, String>>,
    pub(super) proposed_plan: Option<String>,
    pub(super) command: Option<Value>,
}

pub(super) struct DynamicToolCall {
    pub(super) tool: String,
    pub(super) arguments: Value,
}

pub(super) struct RequestBinding {
    pub(super) file_approval: Option<(String, ActivityApproval)>,
    pub(super) wire_id: Value,
    pub(super) request_activity: ActivityRef,
    pub(super) kind: RequestKind,
    pub(super) responded: bool,
}

#[derive(Clone)]
pub(super) enum RequestKind {
    Approval {
        offered: Vec<Value>,
        explicit: bool,
        command: bool,
    },
    Input(InputQuestions),
}

#[derive(Clone)]
pub(super) struct InputQuestions {
    pub(super) questions: Vec<InputQuestion>,
    pub(super) current: usize,
    pub(super) answers: Map<String, Value>,
    pub(super) drafts: HashMap<String, (Option<u32>, String)>,
    pub(super) capture: Option<Arc<Capture>>,
    pub(super) captured_answers: Vec<Option<(Answer, AnswerResponse)>>,
    /// A failed final secret write has an unknown delivery outcome. Keep the
    /// request bound, but make another response for that request impossible.
    pub(super) secret_delivery_blocked: bool,
    /// Diagnostic tool result never includes the entered value.
    pub(super) probe_only: bool,
}

#[derive(Clone)]
pub(super) struct InputQuestion {
    pub(super) id: String,
    pub(super) prompt: String,
    pub(super) question: String,
    pub(super) options: Vec<String>,
    pub(super) choices: Vec<QuestionChoice>,
    pub(super) is_secret: bool,
}

#[derive(Clone, Copy)]
pub(super) struct WireTurnBinding {
    pub(super) turn: TurnRef,
    pub(super) interrupted: bool,
    pub(super) finished: bool,
}

pub(super) struct Backend<P> {
    pub(super) client: AppServerClient<P>,
    pub(super) initialized: bool,
    pub(super) backend_version: Option<String>,
    pub(super) account: Option<AccountId>,
    pub(super) image_wire_supported: bool,
    pub(super) image_capability: ImageInputCapability,
    pub(super) input_image_history: InputImageHistory,
    pub(super) selected_model: Option<String>,
    pub(super) cwd: String,
    pub(super) read_only_review: bool,
    pub(super) secret_probe_enabled: bool,
    pub(super) model_rebind_target: Option<(AccountId, ModelId)>,
    pub(super) new_session_target: Option<(AccountId, ModelId)>,
    pub(super) session: Option<SessionBinding>,
    pub(super) turns: HashMap<TurnRef, String>,
    pub(super) wire_turns: HashMap<String, WireTurnBinding>,
    pub(super) items: HashMap<String, ItemBinding>,
    pub(super) file_changes: HashMap<(TurnRef, String), ActivityRef>,
    pub(super) terminal_commands: HashMap<String, (TurnRef, Option<String>)>,
    pub(super) plans: HashMap<TurnRef, ActivityRef>,
    pub(super) turn_diffs: HashMap<TurnRef, ActivityRef>,
    pub(super) requests: HashMap<ActivityRequestRef, RequestBinding>,
    pub(super) wire_requests: HashMap<String, ActivityRequestRef>,
    pub(super) turn_errors: HashMap<String, String>,
    pub(super) pending_events: VecDeque<BackendEvent>,
    pub(super) terminal_poll: Option<Result<(), BackendFailure>>,
    pub(super) secret_diagnostics_tainted: bool,
    pub(super) next_activity_id: u64,
    pub(super) next_request_id: u64,
}

fn secret_request_attempted(binding: &RequestBinding) -> bool {
    matches!(&binding.kind, RequestKind::Input(questions)
        if questions.has_secret() && (binding.responded || questions.secret_delivery_blocked))
}

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn new_uninitialized(
        client: AppServerClient<P>,
        cwd: String,
        read_only_review: bool,
        model_rebind_target: Option<(AccountId, ModelId)>,
    ) -> Self {
        Self {
            client,
            initialized: false,
            backend_version: None,
            account: None,
            image_wire_supported: false,
            image_capability: ImageInputCapability::Unknown,
            input_image_history: InputImageHistory::TextOnly,
            selected_model: None,
            cwd,
            read_only_review,
            secret_probe_enabled: false,
            model_rebind_target,
            new_session_target: None,
            session: None,
            turns: HashMap::new(),
            wire_turns: HashMap::new(),
            items: HashMap::new(),
            file_changes: HashMap::new(),
            terminal_commands: HashMap::new(),
            plans: HashMap::new(),
            turn_diffs: HashMap::new(),
            requests: HashMap::new(),
            wire_requests: HashMap::new(),
            turn_errors: HashMap::new(),
            pending_events: VecDeque::new(),
            terminal_poll: None,
            secret_diagnostics_tainted: false,
            next_activity_id: 1,
            next_request_id: 1,
        }
    }

    pub(super) fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
            .with_steer()
            .with_native_model_rebind()
            .with_image_input(self.image_capability)
    }

    pub(super) fn secret_dispatch_attempted(&self) -> bool {
        self.secret_diagnostics_tainted || self.requests.values().any(secret_request_attempted)
    }

    pub(super) fn secret_dispatch_attempted_for_turn(&self, turn: TurnRef) -> bool {
        self.secret_diagnostics_tainted
            || self.requests.values().any(|binding| {
                binding.request_activity.turn() == turn && secret_request_attempted(binding)
            })
    }

    pub(super) fn mark_secret_diagnostics_tainted(&mut self) {
        self.secret_diagnostics_tainted = true;
        self.client.mark_secret_diagnostics_tainted();
    }

    pub(super) fn apply_thread_policy(&self, params: &mut Value) {
        if self.read_only_review {
            params["approvalPolicy"] = json!("never");
            params["sandbox"] = json!("read-only");
        }
    }

    pub(super) fn apply_turn_policy(&self, params: &mut Value) {
        if self.read_only_review {
            params["approvalPolicy"] = json!("never");
            params["sandboxPolicy"] = json!({
                "type": "readOnly",
                "networkAccess": false,
            });
        }
    }

    pub(super) fn binding_identity(
        &self,
        session_id: &str,
        thread_id: &str,
    ) -> Result<BackendIdentity, BackendFailure> {
        Ok(match (self.read_only_review, self.account.as_ref()) {
            (true, Some(account)) => BackendIdentity::new(
                READ_ONLY_BINDING_SCHEMA,
                json!({
                    "accountId": account.as_str(),
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            (true, None) => BackendIdentity::new(
                LEGACY_READ_ONLY_BINDING_SCHEMA,
                json!({
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            (false, Some(account)) => BackendIdentity::new(
                STANDARD_BINDING_SCHEMA,
                json!({
                    "accountId": account.as_str(),
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            (false, None) => BackendIdentity::new(
                LEGACY_STANDARD_BINDING_SCHEMA,
                json!({ "sessionId": session_id, "threadId": thread_id }).to_string(),
            ),
        })
    }

    pub(super) fn binding_identity_for_resume(
        &self,
        source: &BackendBindingEvidence,
        session_id: &str,
        thread_id: &str,
    ) -> Result<BackendIdentity, BackendFailure> {
        Ok(match source.binding_identity().schema() {
            LEGACY_STANDARD_BINDING_SCHEMA => BackendIdentity::new(
                LEGACY_STANDARD_BINDING_SCHEMA,
                json!({ "sessionId": session_id, "threadId": thread_id }).to_string(),
            ),
            LEGACY_READ_ONLY_BINDING_SCHEMA => BackendIdentity::new(
                LEGACY_READ_ONLY_BINDING_SCHEMA,
                json!({
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            STANDARD_BINDING_SCHEMA | READ_ONLY_BINDING_SCHEMA => {
                self.binding_identity(session_id, thread_id)?
            },
            _ => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Session,
                    "Codex durable execution binding schema is unsupported",
                ));
            },
        })
    }

    pub(super) fn validate_execution_binding(
        &self,
        binding: &BackendBindingEvidence,
    ) -> Result<(), BackendFailure> {
        let identity = binding.binding_identity();
        let profile_matches = if self.read_only_review {
            matches!(
                identity.schema(),
                READ_ONLY_BINDING_SCHEMA | LEGACY_READ_ONLY_BINDING_SCHEMA
            )
        } else {
            matches!(
                identity.schema(),
                STANDARD_BINDING_SCHEMA | LEGACY_STANDARD_BINDING_SCHEMA
            )
        };
        if !profile_matches {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex durable execution profile differs from the requested resume profile",
            ));
        }
        if let Some(account) = binding_account(identity)?
            && self.account.as_ref() != Some(&account)
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex durable binding account differs from the authenticated account",
            ));
        }
        if self.read_only_review {
            let value: Value = serde_json::from_str(identity.value()).map_err(|_| {
                protocol::protocol_failure("Codex read-only review binding is malformed")
            })?;
            if value.get("executionProfile").and_then(Value::as_str)
                != Some(READ_ONLY_REVIEW_PROFILE)
            {
                return Err(protocol::protocol_failure(
                    "Codex read-only review binding has a different execution profile",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn thread_id(&self, session_id: SessionId) -> Result<&str, BackendFailure> {
        self.session
            .as_ref()
            .filter(|binding| binding.yo == session_id)
            .map(|binding| binding.codex.as_str())
            .ok_or_else(|| protocol::protocol_failure("Codex Session binding was not found"))
    }

    pub(super) fn turn_id(&self, turn: TurnRef) -> Result<&str, BackendFailure> {
        self.turns
            .get(&turn)
            .map(String::as_str)
            .ok_or_else(|| protocol::protocol_failure("Codex Turn binding was not found"))
    }

    pub(super) fn next_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, BackendFailure> {
        let id = NonZeroU64::new(self.next_activity_id)
            .map(ActivityId::new)
            .ok_or_else(|| protocol::protocol_failure("Codex Activity id space was exhausted"))?;
        self.next_activity_id = self
            .next_activity_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Codex Activity id space was exhausted"))?;
        Ok(ActivityRef::new(turn, id))
    }

    pub(super) fn next_request(&mut self) -> Result<RequestId, BackendFailure> {
        let id = NonZeroU64::new(self.next_request_id)
            .map(RequestId::new)
            .ok_or_else(|| protocol::protocol_failure("Codex request id space was exhausted"))?;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Codex request id space was exhausted"))?;
        Ok(id)
    }
}
