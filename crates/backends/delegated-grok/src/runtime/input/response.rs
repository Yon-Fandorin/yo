use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, ActivityUpdate,
    BackendCommandEvidence, BackendFailure,
    interview::{AnswerResponse, Capture},
};

use super::super::state::{Backend, InputAnswer, InputBinding, InputQuestions, wire_key};
use crate::{protocol, transport::JsonPeer};

impl<P: JsonPeer> Backend<P> {
    pub(in crate::runtime) fn respond_to_question(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let binding =
            self.inputs.get(&request).cloned().ok_or_else(|| {
                protocol::protocol_failure("Grok user question is no longer pending")
            })?;
        let mut questions = binding.questions.clone();
        let current = questions.current;
        let question = questions
            .questions
            .get(current)
            .cloned()
            .ok_or_else(|| protocol::protocol_failure("Grok user-question index is invalid"))?;
        if let ActivityResponse::PreviousQuestion { choice, draft } = &response {
            if current == 0
                || choice
                    .is_some_and(|choice| choice == 0 || choice as usize > question.choices.len())
            {
                return Err(protocol::protocol_failure(
                    "previous Grok question is unavailable or draft choice is invalid",
                ));
            }
            questions.drafts[current] = (*choice, draft.as_str().to_owned());
            questions.current -= 1;
            if questions
                .question_profile(questions.current)
                .to_snapshot()
                .is_none()
            {
                return Err(protocol::protocol_failure(
                    "previous Grok question exceeds the display limit",
                ));
            }
            return self.advance_question(request, binding, questions, None);
        }

        let response_activity = self.next_activity(request.activity().turn())?;
        let captured_response = response.clone();
        let (answer, draft, receipt_answer, receipt_notes) = match response {
            ActivityResponse::UserInput(input) => {
                let text = input.as_str();
                let selected = text
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|ordinal| ordinal.checked_sub(1))
                    .filter(|index| *index < question.choices.len());
                if let Some(index) = selected {
                    let selected = &question.choices[index];
                    (
                        InputAnswer {
                            label: selected.label.clone(),
                            preview: question.previews[index].clone(),
                            notes: None,
                        },
                        (None, text.to_owned()),
                        selected.label.clone(),
                        None,
                    )
                } else {
                    if text.trim().is_empty() {
                        return Err(protocol::protocol_failure(
                            "Grok user-question free-text answer is empty",
                        ));
                    }
                    (
                        InputAnswer {
                            label: "Other".to_owned(),
                            preview: None,
                            notes: Some(text.to_owned()),
                        },
                        (None, text.to_owned()),
                        "Other".to_owned(),
                        Some(text.to_owned()),
                    )
                }
            },
            ActivityResponse::QuestionAnswer { choice, notes } => {
                let selected_index = choice
                    .checked_sub(1)
                    .map(|index| index as usize)
                    .filter(|index| *index < question.choices.len())
                    .ok_or_else(|| {
                        protocol::protocol_failure(
                            "Grok question choice is outside the outstanding options",
                        )
                    })?;
                let selected = &question.choices[selected_index];
                let draft_notes = notes.as_str();
                let notes = draft_notes.trim();
                (
                    InputAnswer {
                        label: selected.label.clone(),
                        preview: question.previews[selected_index].clone(),
                        notes: (!notes.is_empty()).then(|| notes.to_owned()),
                    },
                    (Some(choice), draft_notes.to_owned()),
                    selected.label.clone(),
                    (!notes.is_empty()).then(|| notes.to_owned()),
                )
            },
            ActivityResponse::Approval(_)
            | ActivityResponse::PreviousQuestion { .. }
            | ActivityResponse::SecretInput(_)
            | ActivityResponse::SecretInputSubmitted => {
                return Err(protocol::protocol_failure(
                    "response kind does not match the Grok user question",
                ));
            },
        };
        if let Some(Capture::Batch {
            questions: captured,
            ..
        }) = &questions.capture
        {
            let captured_answer = captured[current]
                .project_response(&captured_response)
                .map_err(|error| protocol::protocol_failure(error.to_string()))?;
            questions.captured_answers[current] = Some((
                captured_answer,
                AnswerResponse {
                    question_id: question.id.clone(),
                    request,
                    response_activity,
                },
            ));
        }
        questions.answers[current] = Some(answer);
        questions.drafts[current] = draft;
        let receipt = questions.receipt(&receipt_answer, receipt_notes.as_deref());
        questions.current += 1;
        if questions.current < questions.questions.len() {
            return self.advance_question(
                request,
                binding,
                questions,
                Some((response_activity, receipt)),
            );
        }

        let payload = questions.accepted_payload()?;
        let final_receipt = if let Some(Capture::Batch {
            interview,
            revision,
            ..
        }) = &questions.capture
        {
            match questions
                .captured_answers
                .iter()
                .cloned()
                .collect::<Option<Vec<_>>>()
            {
                Some(pairs) => {
                    let (answers, answer_responses) = pairs.into_iter().unzip();
                    Capture::AcceptedAnswers {
                        interview: *interview,
                        revision: revision.clone(),
                        answers,
                        answer_responses,
                        final_request: request,
                        response_activity,
                    }
                    .to_snapshot()
                    .unwrap_or_else(|error| {
                        questions.recovery_failure_receipt(&error.to_string(), &receipt)
                    })
                },
                None => questions
                    .recovery_failure_receipt("ordered actual answers unavailable", &receipt),
            }
        } else {
            receipt
        };
        self.client.respond(binding.wire_id.clone(), payload)?;
        self.inputs.remove(&request);
        self.wire_inputs.retain(|_, bound| *bound != request);
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityStarted {
                activity: response_activity,
                kind: ActivityKind::UserInputResponse {
                    request_id: request.request_id(),
                },
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityUpdated {
                activity: response_activity,
                update: ActivityUpdate::TextSnapshot(final_receipt),
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityFinished {
                activity: response_activity,
                outcome: ActivityOutcome::Completed,
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityFinished {
                activity: binding.activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(BackendCommandEvidence::None)
    }

    fn advance_question(
        &mut self,
        request: ActivityRequestRef,
        binding: InputBinding,
        questions: InputQuestions,
        response: Option<(yo_core::ActivityRef, String)>,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let activity = self.next_activity(request.activity().turn())?;
        let request_id = self.next_request()?;
        let successor = ActivityRequestRef::new(activity, request_id);
        let summary = questions.prompt();
        let restored_profile = if questions.capture.is_some()
            && (questions.drafts[questions.current].0.is_some()
                || !questions.drafts[questions.current].1.is_empty())
        {
            Some(
                questions
                    .question_profile(questions.current)
                    .to_snapshot()
                    .ok_or_else(|| {
                        protocol::protocol_failure(
                            "restored Grok question exceeds the display limit",
                        )
                    })?,
            )
        } else {
            None
        };
        let wire_key = wire_key(&binding.wire_id)?;
        self.inputs.remove(&request);
        self.inputs.insert(
            successor,
            InputBinding {
                wire_id: binding.wire_id,
                tool_call_id: binding.tool_call_id,
                activity,
                questions,
            },
        );
        self.wire_inputs.insert(wire_key, successor);
        if let Some((response_activity, receipt)) = response {
            self.pending_events
                .push_back(yo_core::BackendEvent::ActivityStarted {
                    activity: response_activity,
                    kind: ActivityKind::UserInputResponse {
                        request_id: request.request_id(),
                    },
                });
            self.pending_events
                .push_back(yo_core::BackendEvent::ActivityUpdated {
                    activity: response_activity,
                    update: ActivityUpdate::TextSnapshot(receipt),
                });
            self.pending_events
                .push_back(yo_core::BackendEvent::ActivityFinished {
                    activity: response_activity,
                    outcome: ActivityOutcome::Completed,
                });
        }
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityFinished {
                activity: binding.activity,
                outcome: ActivityOutcome::Completed,
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(summary),
            });
        if let Some(profile) = restored_profile {
            self.pending_events
                .push_back(yo_core::BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(profile),
                });
        }
        Ok(BackendCommandEvidence::None)
    }
}
