use std::collections::{HashMap, HashSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityKind, ActivityNotice, ActivityOutcome, ActivityQuestion, ActivityRequestRef,
    ActivityResponse, ActivityUpdate, ApprovalDecision, BackendCommandEvidence, BackendEvent,
    BackendFailure, BackendFailureKind, ImageInputCapability, InputImageHistory, ModelInputPart,
    NoticeLevel, QuestionChoice, ToolOutput, UserInput,
    interview::{AnswerResponse, Capture, RECOVERY_UNAVAILABLE_RECEIPT_PREFIX},
};

use super::{
    events,
    state::{Backend, InputQuestion, InputQuestions, RequestBinding, RequestKind},
};
use crate::{observation, protocol};

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn validate_direct_input(
        &mut self,
        input: &UserInput,
    ) -> Result<(), BackendFailure> {
        if input.images().is_empty() && self.input_image_history == InputImageHistory::TextOnly {
            return Ok(());
        }
        let Some(model) = self.selected_model.clone() else {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex image input has no verified selected model",
            ));
        };
        self.require_image_capability(&model)?;
        if let ImageInputCapability::Supported {
            maximum_occurrences,
            maximum_image_bytes,
            maximum_input_bytes,
        } = self.image_capability
            && (!input.images().is_empty())
        {
            let total = input.images().iter().try_fold(0_u64, |total, image| {
                total.checked_add(image.snapshot().png().len() as u64)
            });
            if input.images().len() as u64 > u64::from(maximum_occurrences)
                || input
                    .images()
                    .iter()
                    .any(|image| image.snapshot().png().len() as u64 > maximum_image_bytes)
                || total.is_none_or(|total| total > maximum_input_bytes)
            {
                return Err(BackendFailure::new(
                    BackendFailureKind::InputOverBudget,
                    "Codex image input exceeds the selected model's admitted limits",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn require_image_capability(
        &mut self,
        expected_model: &str,
    ) -> Result<(), BackendFailure> {
        if !self.image_wire_supported {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex image input requires the reviewed image-capable protocol build",
            ));
        }
        if self.selected_model.as_deref() != Some(expected_model)
            || self.image_capability == ImageInputCapability::Unknown
        {
            self.selected_model = Some(expected_model.to_owned());
            self.refresh_model_capability(expected_model);
        }
        match self.image_capability {
            ImageInputCapability::Supported { .. } => Ok(()),
            ImageInputCapability::Unsupported => Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "the selected Codex model does not advertise image input",
            )),
            ImageInputCapability::Unknown => Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex image capability could not be established for the selected model",
            )),
        }
    }

    pub(super) fn refresh_model_capability(&mut self, model: &str) {
        self.image_capability = observation::observe_model_capability(
            &mut self.client,
            model,
            self.image_wire_supported,
        );
    }

    pub(super) fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let response_activity = self.next_activity(request.activity().turn())?;
        let binding = self
            .requests
            .get(&request)
            .filter(|binding| !binding.responded)
            .ok_or_else(|| {
                protocol::protocol_failure("response has no unanswered Codex request")
            })?;
        let wire_id = binding.wire_id.clone();
        let mut next = None;
        let mut response_text;
        let mut answer_seal = None;
        let mut seal_failure = None;
        let navigating = matches!(response, ActivityResponse::PreviousQuestion { .. });
        let (payload, response_kind) = match (&binding.kind, response) {
            (
                RequestKind::Approval {
                    offered,
                    explicit,
                    command,
                },
                ActivityResponse::Approval(decision),
            ) => {
                let wire_decision = match decision {
                    ApprovalDecision::Approved => json!("accept"),
                    ApprovalDecision::Declined => json!("decline"),
                    ApprovalDecision::Offered(choice) => choice
                        .checked_sub(1)
                        .and_then(|index| offered.get(index as usize))
                        .filter(|value| events::approval_choice(value, *command).is_some())
                        .cloned()
                        .ok_or_else(|| {
                            protocol::protocol_failure(
                                "approval choice is outside the supported outstanding decisions",
                            )
                        })?,
                };
                // Native default menus do not remove protocol-level accept/decline support
                // from legacy clients; only an explicit server list constrains those replies.
                if *explicit && !offered.contains(&wire_decision) {
                    return Err(protocol::protocol_failure(
                        "approval decision was not offered by the outstanding Codex request",
                    ));
                }
                response_text = match decision {
                    ApprovalDecision::Approved => "Decision: approved".to_owned(),
                    ApprovalDecision::Declined => "Decision: declined".to_owned(),
                    ApprovalDecision::Offered(_) => {
                        let choice = events::approval_choice(&wire_decision, *command)
                            .expect("validated offered decision");
                        format!("Decision: {}\n{}", choice.label, choice.description)
                    },
                };
                (
                    Some(json!({"decision": wire_decision})),
                    ActivityKind::ApprovalResponse {
                        request_id: request.request_id(),
                    },
                )
            },
            (
                RequestKind::Input(questions),
                ActivityResponse::PreviousQuestion { choice, draft },
            ) => {
                if questions.current == 0
                    || choice.is_some_and(|choice| {
                        choice == 0
                            || choice as usize
                                > questions.questions[questions.current].options.len()
                    })
                {
                    return Err(protocol::protocol_failure(
                        "previous question is unavailable or draft choice is invalid",
                    ));
                }
                let mut questions = questions.clone();
                questions.drafts.insert(
                    questions.questions[questions.current].id.clone(),
                    (choice, draft.as_str().to_owned()),
                );
                if questions.capture.is_none()
                    && (questions
                        .question_profile(questions.current)
                        .to_snapshot()
                        .is_none()
                        || questions
                            .question_profile(questions.current - 1)
                            .to_snapshot()
                            .is_none())
                {
                    return Err(protocol::protocol_failure(
                        "question draft exceeds the presentation limit",
                    ));
                }
                questions.current -= 1;
                next = Some(questions);
                response_text = String::new();
                (
                    None,
                    ActivityKind::UserInputResponse {
                        request_id: request.request_id(),
                    },
                )
            },
            (
                RequestKind::Input(questions),
                response @ (ActivityResponse::UserInput(_)
                | ActivityResponse::QuestionAnswer { .. }),
            ) => {
                let mut questions = questions.clone();
                let question = &questions.questions[questions.current];
                if let Some(Capture::Batch {
                    questions: captured,
                    ..
                }) = questions.capture.as_deref()
                {
                    let answer = captured[questions.current]
                        .project_response(&response)
                        .map_err(|error| protocol::protocol_failure(error.to_string()))?;
                    questions.captured_answers[questions.current] = Some((
                        answer,
                        AnswerResponse {
                            question_id: question.id.clone(),
                            request,
                            response_activity,
                        },
                    ));
                }
                let draft = match &response {
                    ActivityResponse::UserInput(input) => (None, input.as_str().to_owned()),
                    ActivityResponse::QuestionAnswer { choice, notes } => {
                        (Some(*choice), notes.as_str().to_owned())
                    },
                    _ => unreachable!("input response matched above"),
                };
                let (answers, receipt) = match response {
                    ActivityResponse::UserInput(answer) => {
                        let selected = answer
                            .as_str()
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| index.checked_sub(1))
                            .and_then(|index| question.options.get(index));
                        let answer = selected.map_or(answer.as_str(), String::as_str);
                        (vec![answer.to_owned()], questions.receipt(answer, None))
                    },
                    ActivityResponse::QuestionAnswer { choice, notes } => {
                        let selected = usize::try_from(choice)
                            .ok()
                            .and_then(|index| index.checked_sub(1))
                            .and_then(|index| question.options.get(index))
                            .ok_or_else(|| {
                                protocol::protocol_failure(
                                    "question choice is outside the outstanding options",
                                )
                            })?;
                        let mut answers = vec![selected.clone()];
                        if !notes.as_str().trim().is_empty() {
                            answers.push(format!("user_note: {}", notes.as_str().trim()));
                        }
                        let receipt = questions.receipt(
                            selected,
                            Some(notes.as_str().trim()).filter(|notes| !notes.is_empty()),
                        );
                        (answers, receipt)
                    },
                    ActivityResponse::Approval(_) | ActivityResponse::PreviousQuestion { .. } => {
                        unreachable!("input response matched above")
                    },
                };
                questions.drafts.insert(question.id.clone(), draft);
                response_text = receipt;
                questions
                    .answers
                    .insert(question.id.clone(), json!({"answers": answers}));
                questions.current += 1;
                let payload = if questions.current == questions.questions.len() {
                    if let Some(Capture::Batch {
                        interview,
                        revision,
                        ..
                    }) = questions.capture.as_deref()
                    {
                        let pairs = questions
                            .captured_answers
                            .iter()
                            .cloned()
                            .collect::<Option<Vec<_>>>();
                        if let Some(pairs) = pairs {
                            let (answers, answer_responses) = pairs.into_iter().unzip();
                            let seal = Capture::AcceptedAnswers {
                                interview: *interview,
                                revision: revision.clone(),
                                answers,
                                answer_responses,
                                final_request: request,
                                response_activity,
                            }
                            .to_snapshot();
                            match seal {
                                Ok(snapshot) => answer_seal = Some(snapshot),
                                Err(error) => seal_failure = Some(error.to_string()),
                            }
                        } else {
                            seal_failure = Some("ordered actual answers unavailable".into());
                        }
                    }
                    Some(json!({"answers": questions.answers}))
                } else {
                    next = Some(questions);
                    None
                };
                (
                    payload,
                    ActivityKind::UserInputResponse {
                        request_id: request.request_id(),
                    },
                )
            },
            _ => {
                return Err(protocol::protocol_failure(
                    "response kind does not match the Codex request",
                ));
            },
        };
        let interview_progress = match &binding.kind {
            RequestKind::Input(questions) => {
                Some((questions.answers.len(), questions.questions.len()))
            },
            RequestKind::Approval { .. } => None,
        };
        let next = next
            .map(|questions| {
                let activity = self.next_activity(request.activity().turn())?;
                let request_id = self.next_request()?;
                Ok::<_, BackendFailure>((
                    questions,
                    ActivityRequestRef::new(activity, request_id),
                    events::wire_key(&wire_id)?,
                ))
            })
            .transpose()?;
        if let Some(payload) = payload {
            self.client.respond(wire_id.clone(), payload).map_err(|failure| {
                    if let Some((recorded, total)) = interview_progress {
                        BackendFailure::new(
                            failure.kind(),
                            format!(
                                "{}\nInterview incomplete: {recorded}/{total} earlier answers recorded. Final submission was not confirmed.",
                                failure.message()
                            ),
                        )
                    } else {
                        failure
                    }
                })?;
            self.requests
                .get_mut(&request)
                .expect("validated request")
                .responded = true;
            if let Some(seal) = answer_seal {
                response_text = seal;
            } else if let Some(error) = seal_failure {
                response_text = format!(
                    "{} {error}\n{response_text}",
                    RECOVERY_UNAVAILABLE_RECEIPT_PREFIX
                );
            }
        }
        if !navigating {
            self.pending_events
                .push_back(BackendEvent::ActivityStarted {
                    activity: response_activity,
                    kind: response_kind,
                });
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity: response_activity,
                    update: ActivityUpdate::TextSnapshot(response_text),
                });
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: response_activity,
                    outcome: ActivityOutcome::Completed,
                });
        }
        if let Some((questions, successor, wire_key)) = next {
            let activity = successor.activity();
            let request_id = successor.request_id();
            self.requests.remove(&request);
            self.wire_requests.insert(wire_key, successor);
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: request.activity(),
                    outcome: ActivityOutcome::Completed,
                });
            self.pending_events
                .push_back(BackendEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::UserInputRequest { request_id },
                });
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(questions.prompt()),
                });
            self.requests.insert(
                successor,
                RequestBinding {
                    file_approval: None,
                    wire_id,
                    request_activity: activity,
                    kind: RequestKind::Input(questions),
                    responded: false,
                },
            );
        }
        Ok(BackendCommandEvidence::None)
    }
}

pub(super) fn project_input(input: &UserInput) -> Result<Vec<Value>, BackendFailure> {
    if input.images().is_empty() {
        return Ok(vec![json!({
            "type": "text",
            "text": input.model_input(),
        })]);
    }
    let parts = input.model_parts();
    ModelInputPart::validate_user_parts(&parts).map_err(|detail| {
        protocol::protocol_failure(format!("invalid Codex image input: {detail}"))
    })?;
    Ok(parts
        .into_iter()
        .map(|part| match part {
            ModelInputPart::Text { text } => json!({ "type": "text", "text": text }),
            ModelInputPart::Image { snapshot } => json!({
                "type": "image",
                "url": format!("data:image/png;base64,{}", STANDARD.encode(snapshot.png())),
            }),
        })
        .collect())
}

impl InputQuestions {
    pub(super) fn parse(params: &Value) -> Result<Self, BackendFailure> {
        let questions = params
            .get("questions")
            .and_then(Value::as_array)
            .filter(|questions| !questions.is_empty())
            .ok_or_else(|| protocol::protocol_failure("user-input request has no questions"))?;
        let mut ids = HashSet::new();
        let mut parsed = Vec::with_capacity(questions.len());
        for question in questions {
            if question
                .get("isSecret")
                .is_some_and(|secret| secret != &Value::Bool(false))
            {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "secret input requires a secure editor; yo will not echo it into the transcript",
                ));
            }
            let other = match question.get("isOther") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err(protocol::protocol_failure(
                        "question isOther must be boolean",
                    ));
                },
            };
            let id = protocol::string_at(question, &["id"])?;
            if id.is_empty() || !ids.insert(id) {
                return Err(protocol::protocol_failure(
                    "user-input question IDs must be nonempty and unique",
                ));
            }
            let header = protocol::string_at(question, &["header"])?;
            let body = protocol::string_at(question, &["question"])?;
            let mut prompt = format!("{header}\n\n{body}");
            let mut options = Vec::new();
            let mut choices = Vec::new();
            if let Some(values) = question.get("options").filter(|value| !value.is_null()) {
                let values = values.as_array().ok_or_else(|| {
                    protocol::protocol_failure("question options must be an array")
                })?;
                for value in values {
                    let label = protocol::string_at(value, &["label"])?;
                    let description = protocol::string_at(value, &["description"])?;
                    options.push(label.to_owned());
                    choices.push(QuestionChoice {
                        label: label.to_owned(),
                        description: description.to_owned(),
                    });
                    prompt.push_str(&format!("\n{}. {label} — {description}", options.len()));
                }
            }
            if other && !options.is_empty() {
                let label = "None of the above";
                let description = "Choose an answer outside this list.";
                options.push(label.to_owned());
                choices.push(QuestionChoice {
                    label: label.to_owned(),
                    description: description.to_owned(),
                });
                prompt.push_str(&format!("\n{}. {label} — {description}", options.len()));
            }
            parsed.push(InputQuestion {
                id: id.to_owned(),
                prompt,
                question: body.to_owned(),
                options,
                choices,
            });
        }
        Ok(Self {
            captured_answers: vec![None; parsed.len()],
            questions: parsed,
            current: 0,
            answers: Map::new(),
            drafts: HashMap::new(),
            capture: None,
        })
    }

    pub(super) fn incomplete_notice(&self, reason: &str) -> String {
        let recorded = self.answers.len();
        let remaining = self.questions.len().saturating_sub(recorded);
        let base = format!(
            "{recorded}/{} answers recorded · {remaining} unanswered\nSubmission incomplete.\n{reason}",
            self.questions.len()
        );
        let fallback = || {
            ActivityNotice {
                level: NoticeLevel::Warning,
                title: "Interview incomplete".to_owned(),
                message: format!(
                    "{base}\nQuestion list omitted: summary exceeds the output limit."
                ),
            }
            .to_snapshot()
            .expect("bounded summary counts and static reason")
        };
        let mut message = base.clone();
        for (index, question) in self.questions.iter().enumerate() {
            let state = if self.answers.contains_key(&question.id) {
                "Recorded"
            } else {
                "Unanswered"
            };
            let prefix = format!("\n{}. {state}: ", index + 1);
            if message
                .len()
                .checked_add(prefix.len())
                .and_then(|size| size.checked_add(question.question.len()))
                .is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES)
            {
                return fallback();
            }
            message.push_str(&prefix);
            message.push_str(&question.question);
        }
        ActivityNotice {
            level: NoticeLevel::Warning,
            title: "Interview incomplete".to_owned(),
            message,
        }
        .to_snapshot()
        .unwrap_or_else(fallback)
    }

    pub(super) fn receipt(&self, answer: &str, notes: Option<&str>) -> String {
        let progress = format!(
            "Question {} of {}\n",
            self.current + 1,
            self.questions.len()
        );
        let delivery = if self.current + 1 == self.questions.len() {
            "All question responses sent."
        } else {
            "Recorded; waiting for the remaining questions."
        };
        let question = &self.questions[self.current].question;
        let parts = [
            progress.as_str(),
            question,
            "\n\nAnswer: ",
            answer,
            if notes.is_some() { "\nNote: " } else { "" },
            notes.unwrap_or(""),
            "\n\n",
            delivery,
        ];
        let size = parts
            .iter()
            .try_fold(0_usize, |size, part| size.checked_add(part.len()));
        if size.is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES) {
            return format!(
                "{progress}{delivery}\nAnswer display omitted: receipt exceeds the output limit. The original response remains in the session journal."
            );
        }
        let mut text = String::with_capacity(size.expect("bounded receipt size"));
        for part in parts {
            text.push_str(part);
        }
        text
    }

    pub(super) fn question_profile(&self, index: usize) -> ActivityQuestion {
        let question = &self.questions[index];
        let hint = if question.options.is_empty() {
            "Enter your answer."
        } else {
            "Enter a number or write your answer."
        };
        let submission = if self.questions.len() == 1 {
            "Submitting this answer sends your response.".to_owned()
        } else if index + 1 == self.questions.len() {
            format!(
                "Submitting this answer sends all {} answers.",
                self.questions.len()
            )
        } else {
            "This answer is recorded locally. All answers are sent after the final question."
                .to_owned()
        };
        let plain_text = format!(
            "Question {} of {} · {}\n\n{hint}\n{submission}\nEsc interrupts the turn.",
            index + 1,
            self.questions.len(),
            question.prompt
        );
        ActivityQuestion {
            allow_notes: true,
            previous_question: index > 0,
            draft: self.drafts.get(&question.id).map(|(_, text)| text.clone()),
            draft_choice: self
                .drafts
                .get(&question.id)
                .and_then(|(choice, _)| *choice),
            plain_text,
            choices: question.choices.clone(),
        }
    }

    pub(super) fn prompt(&self) -> String {
        if let Some(Capture::Batch {
            interview,
            revision,
            questions,
            ..
        }) = self.capture.as_deref()
        {
            let capture = if self.current == 0 && self.answers.is_empty() && self.drafts.is_empty()
            {
                self.capture.as_deref().cloned().expect("present capture")
            } else {
                Capture::Question {
                    interview: *interview,
                    revision: revision.clone(),
                    question: questions[self.current].clone(),
                }
            };
            if let Ok(snapshot) = capture.to_snapshot() {
                return snapshot;
            }
        }
        let mut profile = self.question_profile(self.current);
        profile.previous_question = self.current > 0
            && self
                .question_profile(self.current - 1)
                .to_snapshot()
                .is_some();
        let mut text = profile.to_snapshot().unwrap_or(profile.plain_text);
        if self.capture.is_none() {
            // Legacy/oversized batches still work live; unseen questions cannot be reconstructed.
            text = text.replace(
                "Esc interrupts the turn.",
                "Complete interview recovery is unavailable. Esc interrupts the turn.",
            );
        }
        text
    }
}
