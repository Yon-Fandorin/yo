use std::collections::HashSet;

use serde_json::{Value, json};
use yo_core::{
    ActivityKind, ActivityUpdate, BackendEvent, BackendFailure, BackendFailureKind, QuestionChoice,
    interview::{Capture, InterviewOption, InterviewQuestion},
};

use super::super::state::{Backend, InputBinding, InputQuestion, InputQuestions, wire_key};
use crate::{protocol, transport::JsonPeer};

const MAX_QUESTIONS: usize = 64;
const MAX_CHOICES: usize = 64;

pub(super) fn map_server_request<P: JsonPeer>(
    backend: &mut Backend<P>,
    wire_id: Value,
    params: Value,
) -> Result<Option<BackendEvent>, BackendFailure> {
    backend.map_question_request(wire_id, params)
}

impl<P: JsonPeer> Backend<P> {
    fn map_question_request(
        &mut self,
        wire_id: Value,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        if let Err(failure) = self.validate_session(&params) {
            self.client.reject(
                wire_id,
                -32602,
                "Grok user question targets an invalid Session",
            )?;
            return Err(failure);
        }
        if self.read_only_review {
            self.client
                .respond(wire_id, json!({ "outcome": "cancelled" }))?;
            return Err(protocol::protocol_failure(
                "read-only delegated review does not accept Grok user questions",
            ));
        }
        let turn = match self.active_turn() {
            Ok(turn) => turn,
            Err(failure) => {
                self.client
                    .respond(wire_id, json!({ "outcome": "cancelled" }))?;
                return Err(failure);
            },
        };
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.interrupt_requested)
        {
            self.client
                .respond(wire_id, json!({ "outcome": "cancelled" }))?;
            return Ok(None);
        }
        let (tool_call_id, questions) = match decode_questions(&params) {
            Ok(decoded) => decoded,
            Err(failure) if failure.kind() == BackendFailureKind::Unsupported => {
                self.client
                    .respond(wire_id, json!({ "outcome": "cancelled" }))?;
                return Err(failure);
            },
            Err(failure) => {
                self.client
                    .reject(wire_id, -32602, "Grok user-question request is invalid")?;
                return Err(failure);
            },
        };

        if self.input_tool_turns.contains_key(&tool_call_id)
            || self.seen_tool_ids.contains(&tool_call_id)
                && self
                    .tools
                    .get(&tool_call_id)
                    .is_none_or(|binding| binding.activity.turn() != turn || binding.finished)
        {
            self.client
                .respond(wire_id, json!({ "outcome": "cancelled" }))?;
            return Err(protocol::protocol_failure(format!(
                "duplicate or stale Grok user-question tool call `{tool_call_id}`"
            )));
        }
        if self.input_tool_turns.len() >= Self::MAX_SESSION_TOOL_IDS {
            self.client
                .respond(wire_id, json!({ "outcome": "cancelled" }))?;
            return Err(protocol::protocol_failure(format!(
                "Grok ACP exceeded the per-Session user-question ToolCallId limit of {}",
                Self::MAX_SESSION_TOOL_IDS
            )));
        }

        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = yo_core::ActivityRequestRef::new(activity, request_id);
        let capture = Capture::batch(
            request,
            questions
                .iter()
                .map(|question| InterviewQuestion {
                    id: question.id.clone(),
                    prompt: question.text.clone(),
                    question: question.text.clone(),
                    options: question
                        .choices
                        .iter()
                        .enumerate()
                        .map(|(index, choice)| InterviewOption {
                            id: (index + 1).to_string(),
                            label: choice.label.clone(),
                            description: choice.description.clone(),
                        })
                        .collect(),
                    allow_free_text: true,
                    allow_notes: true,
                    is_secret: false,
                })
                .collect(),
        )
        .ok();
        let questions = InputQuestions {
            answers: vec![None; questions.len()],
            drafts: vec![(None, String::new()); questions.len()],
            captured_answers: vec![None; questions.len()],
            questions,
            current: 0,
            capture,
        };
        if (0..questions.questions.len())
            .any(|index| questions.question_profile(index).to_snapshot().is_none())
        {
            self.client.reject(
                wire_id,
                -32602,
                "Grok user question exceeds the display limit",
            )?;
            return Err(protocol::protocol_failure(
                "Grok user question exceeds the display limit",
            ));
        }
        let wire_key = wire_key(&wire_id)?;
        if self.wire_approvals.contains_key(&wire_key) || self.wire_inputs.contains_key(&wire_key) {
            return Err(protocol::protocol_failure(
                "duplicate Grok ACP client request id",
            ));
        }
        let summary = questions.prompt();
        self.inputs.insert(
            request,
            InputBinding {
                wire_id,
                tool_call_id: tool_call_id.clone(),
                activity,
                questions,
            },
        );
        self.wire_inputs.insert(wire_key, request);
        self.input_tool_turns.insert(tool_call_id, turn);
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(summary),
            });
        Ok(self.pending_events.pop_front())
    }
}

fn decode_questions(params: &Value) -> Result<(String, Vec<InputQuestion>), BackendFailure> {
    let tool_call_id = super::super::state::identifier_at(params, "toolCallId")?.to_owned();
    match params.get("mode").and_then(Value::as_str) {
        Some("default" | "plan") => {},
        _ => {
            return Err(protocol::protocol_failure(
                "Grok user-question mode is unsupported",
            ));
        },
    }
    let values = params
        .get("questions")
        .and_then(Value::as_array)
        .filter(|questions| !questions.is_empty() && questions.len() <= MAX_QUESTIONS)
        .ok_or_else(|| {
            protocol::protocol_failure("Grok user-question batch must contain 1 to 64 questions")
        })?;
    let mut seen_questions = HashSet::new();
    let mut questions = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let text = value
            .get("question")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                protocol::protocol_failure("Grok user question has no nonempty question text")
            })?;
        if !seen_questions.insert(text) {
            return Err(protocol::protocol_failure(
                "Grok user-question text must be unique",
            ));
        }
        optional_string(value, "id", "Grok user-question id must be a string")?;
        let multi_select = match (value.get("multiSelect"), value.get("multi_select")) {
            (Some(_), Some(_)) => {
                return Err(protocol::protocol_failure(
                    "Grok user question repeats the multi-select field",
                ));
            },
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        };
        match multi_select {
            None | Some(Value::Null | Value::Bool(false)) => {},
            Some(Value::Bool(true)) => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "Grok multi-select user questions are not supported by the Yo interview UI",
                ));
            },
            Some(_) => {
                return Err(protocol::protocol_failure(
                    "Grok user-question multiSelect must be boolean",
                ));
            },
        }
        let options = value
            .get("options")
            .and_then(Value::as_array)
            .filter(|options| options.len() <= MAX_CHOICES)
            .ok_or_else(|| {
                protocol::protocol_failure("Grok user question must contain at most 64 options")
            })?;
        let mut choices = Vec::with_capacity(options.len());
        let mut previews = Vec::with_capacity(options.len());
        for option in options {
            let label = option
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .ok_or_else(|| {
                    protocol::protocol_failure("Grok user-question option has no nonempty label")
                })?;
            let description = option
                .get("description")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    protocol::protocol_failure("Grok user-question option has no description")
                })?;
            let preview = optional_string(
                option,
                "preview",
                "Grok user-question option preview must be a string",
            )?;
            optional_string(
                option,
                "id",
                "Grok user-question option id must be a string",
            )?;
            let description = preview.map_or_else(
                || description.to_owned(),
                |preview| format!("{description}\n\nPreview:\n{preview}"),
            );
            choices.push(QuestionChoice {
                label: label.to_owned(),
                description,
            });
            previews.push(preview.map(str::to_owned));
        }
        questions.push(InputQuestion {
            id: format!("grok-question-{}", index + 1),
            text: text.to_owned(),
            choices,
            previews,
        });
    }
    Ok((tool_call_id, questions))
}

fn optional_string<'a>(
    object: &'a Value,
    field: &str,
    error: &'static str,
) -> Result<Option<&'a str>, BackendFailure> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(protocol::protocol_failure(error)),
    }
}
