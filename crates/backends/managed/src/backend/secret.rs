use serde::Deserialize;
use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityQuestion, ActivityRequestRef, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, FunctionTool, ModelConnectorInputItem,
    ModelReplayTool, NATIVE_SECRET_INTERACTION_NAME, RequestToolExposure, SecretInput,
    TOOL_SCHEMA_DIALECT,
};

use super::{
    AwaitingSecretInput, NativeModelBackend, PreparedSecretRequest, TurnState, failure,
    replay::replay_input,
};

const DESCRIPTION: &str = "Ask the user for one secret value that Yo sends only to the current provider and model. Use only when the task cannot continue without it.";

fn parameters() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "description": "Short public title for the secret request."
            },
            "question": {
                "type": "string",
                "description": "Public question shown before secret entry."
            },
            "purpose": {
                "type": "string",
                "description": "Public reason the current provider and model need the secret."
            }
        },
        "required": ["title", "question", "purpose"],
        "additionalProperties": false
    })
}

pub(super) fn function_tool() -> Result<FunctionTool, BackendFailure> {
    FunctionTool::new(NATIVE_SECRET_INTERACTION_NAME, DESCRIPTION, parameters())
        .map_err(|error| failure(BackendFailureKind::Initialization, error.to_string()))
}

pub(super) fn replay_tool() -> ModelReplayTool {
    ModelReplayTool::new(
        NATIVE_SECRET_INTERACTION_NAME,
        DESCRIPTION,
        TOOL_SCHEMA_DIALECT,
        parameters(),
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SecretRequestArguments {
    pub(super) title: String,
    pub(super) question: String,
    pub(super) purpose: String,
}

impl SecretRequestArguments {
    pub(super) fn parse(arguments: &str) -> Result<Self, BackendFailure> {
        let parsed: Self = serde_json::from_str(arguments).map_err(|_| {
            failure(
                BackendFailureKind::Protocol,
                "native secret request arguments do not match the exact public schema",
            )
        })?;
        if !valid_title(&parsed.title)
            || !valid_text(&parsed.question)
            || !valid_text(&parsed.purpose)
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "native secret request arguments violate their public text bounds",
            ));
        }
        Ok(parsed)
    }
}

fn valid_title(value: &str) -> bool {
    !value.is_empty() && value.len() <= 80 && !value.chars().any(char::is_control)
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.chars().any(|character| {
            character == '\0'
                || (character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
        })
}

impl NativeModelBackend {
    pub(super) fn open_secret_request(
        &mut self,
        state: &mut TurnState,
    ) -> Result<(), BackendFailure> {
        let call = state.pending_secret_call.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "completed secret interaction has no validated call",
            )
        })?;
        let activity = self.next_activity(state.turn)?;
        let request = ActivityRequestRef::new(activity, self.next_request()?);
        let provider = self.binding.provider_id().as_str().to_owned();
        let model = self.binding.model_id().as_str().to_owned();
        let plain_text = format!(
            "{}\n\n{}\n\nPurpose: {}\nProvider: {}\nModel: {}\n\nSubmitting sends this value only to the Provider and Model shown above. They may retain it. Submission makes this Session unavailable for later Turns, model replacement, or resume even if transport never starts or no answer arrives. The final answer is withheld until its complete visible text passes an exact secret-echo check.\nEsc interrupts the Turn.",
            call.arguments.title, call.arguments.question, call.arguments.purpose, provider, model,
        );
        let snapshot = ActivityQuestion {
            plain_text,
            choices: Vec::new(),
            allow_notes: false,
            is_secret: true,
            previous_question: false,
            draft: None,
            draft_choice: None,
        }
        .to_snapshot()
        .ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "native secret request presentation exceeds its public bound",
            )
        })?;
        self.queue_activity_text(
            activity,
            ActivityKind::UserInputRequest {
                request_id: request.request_id(),
            },
            snapshot,
            None,
        );
        state.awaiting_secret_input = Some(AwaitingSecretInput {
            request,
            call_id: call.call_id,
        });
        Ok(())
    }

    pub(super) fn respond_to_secret_input(
        &mut self,
        request: ActivityRequestRef,
        secret: SecretInput,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "secret response has no active Turn",
            )
        })?;
        let Some(awaiting) = state.awaiting_secret_input.take() else {
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "no native secret request is awaiting a response",
            ));
        };
        if awaiting.request != request {
            state.awaiting_secret_input = Some(awaiting);
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "secret response does not match the exact native request",
            ));
        }
        let value = secret.into_inner();
        if value.is_empty() {
            state.awaiting_secret_input = Some(awaiting);
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "native secret input must contain at least one UTF-8 byte",
            ));
        }
        let mut items = Vec::new();
        items.push(ModelConnectorInputItem::Message {
            role: yo_core::ModelConnectorInputRole::System,
            content: self.contract.system_prompt().to_owned(),
            refusal: None,
        });
        items.extend(self.replay.items().iter().map(replay_input));
        items.extend(state.delta.iter().map(replay_input));
        items.push(ModelConnectorInputItem::FunctionCallOutput {
            call_id: awaiting.call_id.clone(),
            output: value.clone(),
        });
        let prepared = self.admitted_request(
            items,
            RequestToolExposure::disabled(),
            state.turn.session_id(),
        );
        let (request_payload, _) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                state.awaiting_secret_input = Some(awaiting);
                self.turn = Some(state);
                return Err(error);
            },
        };
        let request_payload = match request_payload.with_protected_terminal_input() {
            Ok(request) => request,
            Err(_) => {
                state.awaiting_secret_input = Some(awaiting);
                self.turn = Some(state);
                return Err(failure(
                    BackendFailureKind::Protocol,
                    "native secret connector request violated its terminal shape",
                ));
            },
        };
        state.prepared_secret_request = Some(PreparedSecretRequest {
            request: Some(request_payload),
            comparison: value,
            armed: false,
        });
        state.terminal_secret_request = true;
        self.events.push_back(BackendEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        });
        let response_activity = self.next_activity(state.turn)?;
        self.queue_activity_text(
            response_activity,
            ActivityKind::UserInputResponse {
                request_id: request.request_id(),
            },
            "Secret submitted to the disclosed Provider and Model. The value was not recorded by Yo.".to_owned(),
            Some(ActivityOutcome::Completed),
        );
        self.turn = Some(state);
        Ok(BackendCommandEvidence::ProtectedInputPrepared)
    }

    pub(super) fn commit_secret_request(&mut self) -> Result<(), BackendFailure> {
        let prepared = self
            .turn
            .as_mut()
            .and_then(|state| state.prepared_secret_request.as_mut())
            .ok_or_else(|| {
                failure(
                    BackendFailureKind::Protocol,
                    "no protected request is prepared for durable commit",
                )
            })?;
        if prepared.armed || prepared.request.is_none() {
            return Err(failure(
                BackendFailureKind::Protocol,
                "protected request was already committed",
            ));
        }
        prepared.armed = true;
        Ok(())
    }

    pub(super) fn abort_secret_request(&mut self) {
        if let Some(state) = self.turn.as_mut() {
            state.prepared_secret_request = None;
        }
        self.events.clear();
    }

    pub(super) fn start_prepared_secret_request(
        &mut self,
        state: &mut TurnState,
    ) -> Result<(), BackendFailure> {
        let prepared = state.prepared_secret_request.as_mut().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "terminal secret request lost its prepared input",
            )
        })?;
        if !prepared.armed {
            return Err(failure(
                BackendFailureKind::Protocol,
                "terminal secret request was polled before durable commit",
            ));
        }
        let request = prepared.request.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "terminal secret request transport was already started",
            )
        })?;
        let cancellation = yo_core::ModelConnectorCancellation::new();
        *self
            .shared_stop
            .response
            .lock()
            .map_err(|_| failure(BackendFailureKind::Cleanup, "native stop state is poisoned"))? =
            Some(cancellation.clone());
        let stream = match self.connector.start(request, cancellation) {
            Ok(stream) => stream,
            Err(_) => {
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;
                return Err(failure(
                    BackendFailureKind::Turn,
                    "terminal secret request failed after submission; delivery outcome is unknown",
                ));
            },
        };
        state.stream = Some(stream);
        state.round += 1;
        state.response_id = None;
        state.assistant_activities.clear();
        state.reasoning_activities.clear();
        state.call_activities.clear();
        state.round_message_items.clear();
        state.round_messages.clear();
        state.round_refusals.clear();
        state.round_replay.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SecretRequestArguments;

    // 공개 인자만 정확한 닫힌 스키마와 UTF-8 바이트·제어문자 경계를 통과하는지 확인한다.
    #[test]
    fn secret_request_arguments_enforce_the_exact_public_schema_and_byte_bounds() {
        let exact_title = "x".repeat(80);
        let exact_question = "q".repeat(4096);
        let exact = serde_json::json!({
            "title": exact_title,
            "question": exact_question,
            "purpose": "Authenticate the current request."
        })
        .to_string();
        assert!(SecretRequestArguments::parse(&exact).is_ok());

        for rejected in [
            r#"{"title":"","question":"Question","purpose":"Purpose"}"#.to_owned(),
            serde_json::json!({
                "title": "x".repeat(81),
                "question": "Question",
                "purpose": "Purpose"
            })
            .to_string(),
            serde_json::json!({
                "title": "Title",
                "question": "q".repeat(4097),
                "purpose": "Purpose"
            })
            .to_string(),
            r#"{"title":"Title","question":"Question"}"#.to_owned(),
            r#"{"title":"Title","question":"Question","purpose":"Purpose","extra":true}"#
                .to_owned(),
            r#"{"title":"Title\nInjected","question":"Question","purpose":"Purpose"}"#.to_owned(),
            r#"{"title":"Title","question":"Question\u0007","purpose":"Purpose"}"#.to_owned(),
        ] {
            assert!(
                SecretRequestArguments::parse(&rejected).is_err(),
                "unexpectedly accepted {rejected}"
            );
        }
    }
}
