//! 일반 질문의 닫힌 인자, 응답 준비와 durable commit 이후 소비를 소유합니다.

use std::{collections::HashSet, iter};

use serde::Deserialize;
use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityQuestion, ActivityRequestRef, ActivityResponse,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, FunctionTool,
    ModelReplayItem, ModelReplayTool, QuestionChoice,
};

use super::{NativeModelBackend, TurnState, failure};

pub(super) const NAME: &str = "ask_user";
const DESCRIPTION: &str = "Ask the user one question and wait for a response. Use for a missing preference or decision. Unanswered provides no answer or permission; answer-dependent decisions remain unresolved.";
fn parameters() -> serde_json::Value {
    json!({"type":"object","properties":{"title":{"type":"string","description":"Short public title for the question."},"question":{"type":"string","description":"Question to show the user."},"choices":{"type":"array","description":"Optional ordered choices; the user can always answer with text.","items":{"type":"object","properties":{"label":{"type":"string","description":"Short choice label."},"description":{"type":"string","description":"Public explanation of the choice."}},"required":["label","description"],"additionalProperties":false}}},"required":["title","question"],"additionalProperties":false})
}
pub(super) fn replay_tool() -> ModelReplayTool {
    ModelReplayTool::new(NAME, DESCRIPTION, "yo.tool-schema/v1", parameters())
}
pub(super) fn function_tool() -> Result<FunctionTool, BackendFailure> {
    FunctionTool::new(NAME, DESCRIPTION, parameters())
        .map_err(|e| failure(BackendFailureKind::Initialization, e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Arguments {
    title: String,
    question: String,
    #[serde(default)]
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    label: String,
    description: String,
}
impl Arguments {
    pub(super) fn parse(raw: &str) -> Result<Self, BackendFailure> {
        let invalid = || {
            failure(
                BackendFailureKind::Protocol,
                "ordinary question arguments violate their closed schema or bounds",
            )
        };
        if raw.len() > 16_384 {
            return Err(invalid());
        }
        let value: Self = serde_json::from_str(raw).map_err(|_| invalid())?;
        let label = |s: &str| !s.is_empty() && s.len() <= 80 && !s.chars().any(char::is_control);
        let text = |s: &str| {
            !s.chars()
                .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        };
        let mut labels = HashSet::new();
        if !label(&value.title)
            || value.question.is_empty()
            || value.question.len() > 4096
            || !text(&value.question)
            || value.choices.len() > 8
            || value.choices.iter().any(|c| {
                !label(&c.label)
                    || !labels.insert(&c.label)
                    || c.description.len() > 512
                    || !text(&c.description)
            })
        {
            return Err(invalid());
        }
        Ok(value)
    }
}
pub(super) struct PendingQuestion {
    pub(super) call_id: String,
    pub(super) arguments: Arguments,
}
pub(super) struct AwaitingQuestion {
    request: ActivityRequestRef,
    call_id: String,
    choices: Vec<Choice>,
}
pub(super) struct PreparedQuestion {
    request: ActivityRequestRef,
    item: ModelReplayItem,
    presentation: String,
    activity: yo_core::ActivityRef,
}

impl NativeModelBackend {
    pub(super) fn open_question(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let call = state
            .pending_question
            .take()
            .expect("validated complete question");
        let activity = self.next_activity(state.turn)?;
        let request = ActivityRequestRef::new(activity, self.next_request()?);
        let snapshot = ActivityQuestion {
            plain_text: format!(
                "{}\n\n{}\n\nEsc: no answer · Ctrl+C: interrupt",
                call.arguments.title, call.arguments.question
            ),
            choices: call
                .arguments
                .choices
                .iter()
                .map(|c| QuestionChoice {
                    label: c.label.clone(),
                    description: c.description.clone(),
                })
                .collect(),
            allow_notes: true,
            allow_unanswered: true,
            is_secret: false,
            storage_offer: None,
            previous_question: false,
            draft: None,
            draft_choice: None,
        }
        .to_snapshot()
        .ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "ordinary question presentation exceeds its bound",
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
        state.awaiting_question = Some(AwaitingQuestion {
            request,
            call_id: call.call_id,
            choices: call.arguments.choices,
        });
        Ok(())
    }
    pub(super) fn respond_to_question(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let state = self.turn.as_ref().ok_or_else(|| {
            failure(
                BackendFailureKind::CommandRejected,
                "ordinary question has no active Turn",
            )
        })?;
        let awaiting = state
            .awaiting_question
            .as_ref()
            .filter(|q| {
                q.request == request
                    && state.prepared_question.is_none()
                    && state.prepared_steer.is_none()
            })
            .ok_or_else(|| {
                failure(
                    BackendFailureKind::CommandRejected,
                    "response does not match the outstanding ordinary question",
                )
            })?;
        let invalid = || {
            failure(
                BackendFailureKind::InputOverBudget,
                "ordinary question response violates its kind or byte bounds",
            )
        };
        let (result, presentation) = match response {
            ActivityResponse::UserInput(input)
                if !input.as_str().is_empty()
                    && input.as_str().len() <= 16_384
                    && input.images().is_empty()
                    && input.resolved_skill().is_none() =>
            {
                (
                    json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"text","text":input.as_str()}),
                    input.as_str().to_owned(),
                )
            },
            ActivityResponse::QuestionAnswer { choice, notes }
                if notes.as_str().len() <= 16_384
                    && notes.images().is_empty()
                    && notes.resolved_skill().is_none() =>
            {
                let selected = choice
                    .checked_sub(1)
                    .and_then(|n| awaiting.choices.get(n as usize))
                    .ok_or_else(invalid)?;
                (
                    json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"choice","choice":choice,"label":selected.label,"notes":notes.as_str()}),
                    format!("{} · {}\n{}", choice, selected.label, notes.as_str()),
                )
            },
            ActivityResponse::QuestionUnanswered => (
                json!({"schema":"yo.ask-user-result/v1","status":"unanswered"}),
                "No answer provided. No decision or permission was supplied.".to_owned(),
            ),
            _ => return Err(invalid()),
        };
        let item = ModelReplayItem::FunctionCallOutput {
            call_id: awaiting.call_id.clone(),
            output: result.to_string(),
        };
        self.ensure_accumulated_replay_capacity(state, Some(&item))?;
        let mut replay = self.replay.clone();
        let delta = yo_core::ModelReplayDelta::new(
            self.replay
                .contract()
                .is_none()
                .then(|| self.contract.clone()),
            state
                .delta
                .iter()
                .chain(iter::once(&item))
                .chain(state.armed_steers.iter())
                .cloned()
                .collect(),
        );
        replay
            .apply(&delta)
            .map_err(|e| failure(BackendFailureKind::ContextExhausted, e))?;
        let turn = state.turn;
        let activity = self.next_activity(turn)?;
        self.turn
            .as_mut()
            .expect("active question")
            .prepared_question = Some(PreparedQuestion {
            request,
            item,
            presentation,
            activity,
        });
        Ok(BackendCommandEvidence::OrdinaryQuestionResponsePrepared)
    }
    pub(super) fn commit_question_response(&mut self) -> Result<(), BackendFailure> {
        let state = self.turn.as_mut().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "no prepared ordinary question response",
            )
        })?;
        let prepared = state.prepared_question.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "no prepared ordinary question response",
            )
        })?;
        state.awaiting_question = None;
        state.delta.push(prepared.item);
        state.start_next_round = true;
        let turn = state.turn;
        let items = state.delta.clone();
        self.events.push_back(BackendEvent::ActivityFinished {
            activity: prepared.request.activity(),
            outcome: ActivityOutcome::Completed,
        });
        self.queue_activity_text(
            prepared.activity,
            ActivityKind::UserInputResponse {
                request_id: prepared.request.request_id(),
            },
            prepared.presentation,
            Some(ActivityOutcome::Completed),
        );
        self.events
            .push_back(BackendEvent::ContextActiveSuffixCompleted { turn, items });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
