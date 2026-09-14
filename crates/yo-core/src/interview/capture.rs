use std::collections::HashMap;

use super::{Answer, AnswerResponse, Capture, InterviewQuestion};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    ActivityUpdate, AgentCommand, AgentEvent, TranscriptRecord,
};

pub(crate) fn initial_submission_evidence(
    entries: &[crate::journal::JournalEntry],
    id: crate::SubmissionId,
    durability: crate::JournalDurability,
) -> Option<(crate::TurnRef, crate::JournalSequence)> {
    use crate::journal::SemanticRecord;
    let (command_sequence, turn, actual_id) =
        entries.iter().find_map(|entry| match entry.record() {
            SemanticRecord::CommandCommitted(command) => match command.command() {
                AgentCommand::StartTurn { turn, .. } => {
                    Some((entry.sequence(), *turn, command.submission_id()))
                },
                _ => None,
            },
            _ => None,
        })?;
    if actual_id != Some(id) {
        return None;
    }
    let cutoff = match durability {
        crate::JournalDurability::Durable {
            journal_sequence, ..
        } => journal_sequence?,
        crate::JournalDurability::Gap {
            durable_cutoff:
                crate::session_repository::DurableCutoff::Known {
                    journal_sequence, ..
                },
            ..
        } => journal_sequence?,
        _ => return None,
    };
    use crate::journal::codec::{ExchangeDirection, ExchangeKind, OperationId};
    entries.iter().find_map(|entry| {
        let SemanticRecord::BackendRequestAccepted(accepted) = entry.record() else {
            return None;
        };
        if accepted.turn_id() != turn.turn_id()
            || accepted.operation_id() != OperationId::from(id)
            || entry.sequence() <= command_sequence
            || entry.sequence() > cutoff
        {
            return None;
        }
        let correlated = entries.iter().any(|exchange| {
            if exchange.sequence() != accepted.exchange_sequence() {
                return false;
            }
            matches!(exchange.record(), SemanticRecord::BackendExchangeObserved(observation)
                if observation.operation_id() == accepted.operation_id()
                    && observation.kind() == ExchangeKind::Request
                    && observation.direction() == ExchangeDirection::YoToBackend)
        });
        correlated.then_some((turn, entry.sequence()))
    })
}

/// Complete validated questions plus genuine successful answer correlations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedInterview {
    pub interview: ActivityRequestRef,
    pub revision: String,
    pub questions: Vec<InterviewQuestion>,
    pub answers: Vec<Answer>,
    pub current_question_id: String,
    pub submitted: Option<(ActivityRequestRef, ActivityRef)>,
}
#[derive(Clone, Debug, Default)]
pub struct InterviewCatalog {
    interviews: Vec<CapturedInterview>,
    kinds: HashMap<ActivityRef, ActivityKind>,
    requests: HashMap<ActivityRequestRef, (usize, usize)>,
    commands: HashMap<ActivityRequestRef, ActivityResponse>,
    responses: HashMap<ActivityRef, (ActivityRequestRef, usize, usize, Answer)>,
    latest: HashMap<(usize, usize), (AnswerResponse, Answer)>,
    seals: HashMap<ActivityRef, Capture>,
    unavailable: HashMap<ActivityRef, (ActivityRequestRef, String)>,
    unavailable_pending: HashMap<ActivityRef, (ActivityRequestRef, String)>,
}
impl InterviewCatalog {
    pub fn is_interview_activity(&self, activity: ActivityRef) -> bool {
        matches!(
            self.kinds.get(&activity),
            Some(ActivityKind::UserInputRequest { .. } | ActivityKind::UserInputResponse { .. })
        )
    }
    fn recovery_unavailable_receipt(&self, activity: ActivityRef, text: &str) -> bool {
        matches!(
            self.kinds.get(&activity),
            Some(ActivityKind::UserInputResponse { .. })
        ) && text.starts_with(super::RECOVERY_UNAVAILABLE_RECEIPT_PREFIX)
    }
    pub fn recovery_unavailable(&self, interview: ActivityRequestRef) -> Option<&str> {
        self.unavailable
            .values()
            .find_map(|(source, text)| (*source == interview).then_some(text.as_str()))
    }
    pub fn completed_answer(
        &self,
        activity: ActivityRef,
    ) -> Option<(ActivityRequestRef, usize, &Answer)> {
        self.latest
            .iter()
            .find_map(|(&(batch, index), (response, answer))| {
                (response.response_activity == activity).then_some((
                    self.interviews[batch].interview,
                    index,
                    answer,
                ))
            })
    }
    pub fn answer_receipt(&self, activity: ActivityRef) -> Option<String> {
        let Capture::AcceptedAnswers {
            interview,
            revision,
            answers,
            answer_responses,
            ..
        } = self.seals.get(&activity)?
        else {
            return None;
        };
        let capture = self.find(*interview, revision)?;
        if answers.len() != capture.questions.len() {
            return None;
        }
        let mut text = String::new();
        for ((question, answer), response) in
            capture.questions.iter().zip(answers).zip(answer_responses)
        {
            question.validate_answer(answer, false).ok()?;
            let actual = self
                .responses
                .get(&activity)
                .filter(|(request, _, _, a)| *request == response.request && a == answer)
                .is_some()
                || self
                    .latest
                    .values()
                    .any(|(r, a)| r == response && a == answer);
            if !actual {
                return None;
            }
            let value = answer
                .option_id
                .as_ref()
                .and_then(|id| question.options.iter().find(|o| &o.id == id))
                .map_or(answer.text.as_str(), |o| o.label.as_str());
            text.push_str(&format!("{}\nAnswer: {value}\n", question.prompt));
            if !answer.notes.is_empty() {
                text.push_str(&format!("Note: {}\n", answer.notes));
            }
        }
        Some(text)
    }
    pub fn question_for_request(
        &self,
        request: ActivityRequestRef,
    ) -> Option<(&CapturedInterview, usize)> {
        let &(batch, index) = self.requests.get(&request)?;
        Some((&self.interviews[batch], index))
    }
    pub fn presentation(&self, activity: ActivityRef) -> Option<crate::ActivityQuestion> {
        let ActivityKind::UserInputRequest { request_id } = *self.kinds.get(&activity)? else {
            return None;
        };
        let (capture, index) =
            self.question_for_request(ActivityRequestRef::new(activity, request_id))?;
        Some(capture.questions[index].presentation(index, capture.questions.len()))
    }
    pub fn interviews(&self) -> &[CapturedInterview] {
        &self.interviews
    }
    pub fn find(
        &self,
        interview: ActivityRequestRef,
        revision: &str,
    ) -> Option<&CapturedInterview> {
        self.interviews
            .iter()
            .find(|v| v.interview == interview && v.revision == revision)
    }
    pub fn observe_committed(&mut self, record: &TranscriptRecord) {
        match record {
            TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
                request,
                response,
            }) => {
                self.commands.insert(*request, response.clone());
            },
            TranscriptRecord::EventCommitted(event) => self.event(event),
            _ => {},
        }
    }
    fn event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ActivityStarted { activity, kind } => {
                self.kinds.insert(*activity, *kind);
                if let ActivityKind::UserInputResponse { request_id } = kind {
                    let request = self
                        .commands
                        .keys()
                        .find(|r| {
                            r.activity().turn() == activity.turn() && r.request_id() == *request_id
                        })
                        .copied();
                    if let Some(request) = request
                        && let Some(&(batch, index)) = self.requests.get(&request)
                        && let Some(response) = self.commands.get(&request)
                        && let Ok(answer) =
                            self.interviews[batch].questions[index].project_response(response)
                    {
                        self.responses
                            .insert(*activity, (request, batch, index, answer));
                    }
                }
            },
            AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            } => {
                self.unavailable_pending.remove(activity);
                if self.recovery_unavailable_receipt(*activity, text)
                    && let Some((_, batch, _, _)) = self.responses.get(activity)
                    && let Some(line) = text.lines().next()
                    && line.len() <= super::RECOVERY_DIAGNOSTIC_LIMIT
                {
                    self.unavailable_pending.insert(
                        *activity,
                        (self.interviews[*batch].interview, line.to_owned()),
                    );
                }
                let Ok(capture) = Capture::from_snapshot(text) else {
                    return;
                };
                match &capture {
                    Capture::Batch {
                        interview,
                        revision,
                        questions,
                        current_question_id,
                    } => {
                        if self.kinds.get(activity)
                            != Some(&ActivityKind::UserInputRequest {
                                request_id: interview.request_id(),
                            })
                            || interview.activity() != *activity
                            || self.interviews.iter().any(|v| v.interview == *interview)
                        {
                            return;
                        }
                        let batch = self.interviews.len();
                        self.interviews.push(CapturedInterview {
                            interview: *interview,
                            revision: revision.clone(),
                            questions: questions.clone(),
                            answers: questions
                                .iter()
                                .map(InterviewQuestion::empty_answer)
                                .collect(),
                            current_question_id: current_question_id.clone(),
                            submitted: None,
                        });
                        self.requests.insert(*interview, (batch, 0));
                    },
                    Capture::Question {
                        interview,
                        revision,
                        question,
                    } => {
                        let Some(ActivityKind::UserInputRequest { request_id }) =
                            self.kinds.get(activity)
                        else {
                            return;
                        };
                        let Some(batch) = self
                            .interviews
                            .iter()
                            .position(|v| v.interview == *interview && v.revision == *revision)
                        else {
                            return;
                        };
                        if activity.turn() != interview.activity().turn() {
                            return;
                        }
                        let Some(index) = self.interviews[batch]
                            .questions
                            .iter()
                            .position(|q| q == question)
                        else {
                            return;
                        };
                        let request = ActivityRequestRef::new(*activity, *request_id);
                        if self.requests.contains_key(&request) {
                            return;
                        }
                        self.requests.insert(request, (batch, index));
                        self.interviews[batch].current_question_id = question.id.clone();
                    },
                    Capture::AcceptedAnswers {
                        response_activity,
                        final_request,
                        ..
                    } => {
                        if response_activity == activity
                            && self.kinds.get(activity)
                                == Some(&ActivityKind::UserInputResponse {
                                    request_id: final_request.request_id(),
                                })
                            && self.responses.contains_key(activity)
                        {
                            self.seals.insert(*activity, capture);
                        }
                    },
                }
            },
            AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextDelta(_),
            } => {
                self.unavailable_pending.remove(activity);
            },
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            } => {
                if let Some((request, batch, index, answer)) = self.responses.remove(activity) {
                    if let Some(diagnostic) = self.unavailable_pending.remove(activity) {
                        self.unavailable.insert(*activity, diagnostic);
                    }
                    self.interviews[batch].answers[index] = answer.clone();
                    self.latest.insert(
                        (batch, index),
                        (
                            AnswerResponse {
                                question_id: answer.question_id.clone(),
                                request,
                                response_activity: *activity,
                            },
                            answer,
                        ),
                    );
                    if let Some(Capture::AcceptedAnswers {
                        interview,
                        revision,
                        answers,
                        answer_responses,
                        final_request,
                        response_activity,
                    }) = self.seals.remove(activity)
                    {
                        let captured = &self.interviews[batch];
                        let valid = captured.interview == interview
                            && captured.revision == revision
                            && final_request == request
                            && response_activity == *activity
                            && answers.len() == captured.questions.len()
                            && answers.iter().zip(&answer_responses).enumerate().all(
                                |(i, (a, r))| {
                                    self.latest.get(&(batch, i)).is_some_and(
                                        |(actual_r, actual_a)| actual_r == r && actual_a == a,
                                    )
                                },
                            );
                        if valid {
                            self.interviews[batch].submitted =
                                Some((final_request, response_activity));
                        }
                    }
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                self.responses.remove(activity);
                self.seals.remove(activity);
                self.unavailable_pending.remove(activity);
                self.unavailable.remove(activity);
            },
            _ => {},
        }
    }
    pub(crate) fn from_records<'a>(
        records: impl IntoIterator<Item = &'a TranscriptRecord>,
    ) -> Self {
        let mut catalog = Self::default();
        for record in records {
            catalog.observe_committed(record);
        }
        catalog
    }
}
