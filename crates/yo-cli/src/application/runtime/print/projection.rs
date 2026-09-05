use std::collections::HashMap;

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent,
    TranscriptObservation, TranscriptRecord, TurnOutcome, TurnRef,
};

use crate::interaction::diagnostic::AppError;

pub(super) fn frame_output(mut message: String) -> String {
    if !message.ends_with('\n') {
        message.push('\n');
    }
    message
}

#[derive(Default)]
pub(super) struct FinalResponseProjection {
    turn: Option<TurnRef>,
    activities: HashMap<ActivityRef, ProjectedActivity>,
    last_completed_agent_message: Option<String>,
}

struct ProjectedActivity {
    kind: ActivityKind,
    text: String,
}

impl FinalResponseProjection {
    pub(super) fn observe(
        &mut self,
        observation: &TranscriptObservation,
    ) -> Result<Option<String>, AppError> {
        let TranscriptObservation::Record(record) = observation else {
            return Ok(None);
        };
        self.observe_record(record)
    }

    fn observe_record(&mut self, record: &TranscriptRecord) -> Result<Option<String>, AppError> {
        match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { turn, .. }) => {
                if self.turn.replace(*turn).is_some() {
                    return Err(AppError::message(
                        "print Session committed more than one started Turn",
                    ));
                }
            },
            TranscriptRecord::CommandCommitted(AgentCommand::SteerTurn { .. }) => {
                return Err(AppError::message(
                    "print Session committed an unexpected steer Submission",
                ));
            },
            TranscriptRecord::CommandCommitted(_) => {},
            TranscriptRecord::EventCommitted(event) => return self.observe_event(event),
            TranscriptRecord::ContextPolicyChanged(_)
            | TranscriptRecord::ContextCheckpointCommitted(_) => {},
        }
        Ok(None)
    }

    fn observe_event(&mut self, event: &AgentEvent) -> Result<Option<String>, AppError> {
        match event {
            AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {},
            AgentEvent::ActivityStarted { activity, kind }
                if self.turn == Some(activity.turn()) =>
            {
                if matches!(
                    kind,
                    ActivityKind::ApprovalRequest { .. } | ActivityKind::UserInputRequest { .. }
                ) {
                    return Err(AppError::message(
                        "print Session requires an interactive response",
                    ));
                }
                if self
                    .activities
                    .insert(
                        *activity,
                        ProjectedActivity {
                            kind: *kind,
                            text: String::new(),
                        },
                    )
                    .is_some()
                {
                    return Err(AppError::message(
                        "print Session started the same Activity more than once",
                    ));
                }
            },
            AgentEvent::ActivityStarted { .. } => {},
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some(activity) = self.activities.get_mut(activity) else {
                    return Err(AppError::message(
                        "print Session updated an unknown Activity",
                    ));
                };
                match update {
                    ActivityUpdate::TextDelta(text) => activity.text.push_str(text),
                    ActivityUpdate::TextSnapshot(text) => activity.text.clone_from(text),
                }
            },
            AgentEvent::ActivityFinished { activity, outcome } => {
                let Some(activity) = self.activities.remove(activity) else {
                    return Err(AppError::message(
                        "print Session finished an unknown Activity",
                    ));
                };
                if activity.kind == ActivityKind::AgentMessage
                    && matches!(outcome, ActivityOutcome::Completed)
                {
                    self.last_completed_agent_message = Some(activity.text);
                }
            },
            AgentEvent::TurnFinished { turn, outcome } if self.turn == Some(*turn) => {
                return match outcome {
                    TurnOutcome::Completed => self
                        .last_completed_agent_message
                        .take()
                        .map(Some)
                        .ok_or_else(|| {
                            AppError::message(
                                "print Turn completed without a completed AgentMessage",
                            )
                        }),
                    TurnOutcome::Interrupted => {
                        Err(AppError::message("print Turn was interrupted"))
                    },
                    TurnOutcome::Failed(failure) => Err(AppError::message(format!(
                        "print Turn failed: {}",
                        failure.message()
                    ))),
                };
            },
            AgentEvent::TurnFinished { .. } => {},
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // projection framing은 정확히 하나의 trailing LF만 보장하고 이미 framing된 AgentMessage
    // bytes에는 두 번째 LF를 추가하지 않습니다.
    #[test]
    fn output_framing_adds_only_a_missing_final_lf() {
        assert_eq!(frame_output("answer".to_owned()), "answer\n");
        assert_eq!(frame_output("answer\n".to_owned()), "answer\n");
        assert_eq!(frame_output("answer\n\n".to_owned()), "answer\n\n");
    }
}
