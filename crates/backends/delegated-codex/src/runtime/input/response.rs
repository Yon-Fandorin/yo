use serde_json::json;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, ActivityUpdate,
    ApprovalDecision, BackendCommandEvidence, BackendEvent, BackendFailure,
    interview::{AnswerResponse, Capture, RECOVERY_UNAVAILABLE_RECEIPT_PREFIX},
};

use super::super::{
    events,
    state::{Backend, RequestBinding, RequestKind},
};
use crate::protocol;

pub(super) fn respond_to_activity<P: JsonMessagePeer>(
    backend: &mut Backend<P>,
    request: ActivityRequestRef,
    response: ActivityResponse,
) -> Result<BackendCommandEvidence, BackendFailure> {
    if matches!(&response, ActivityResponse::SecretInput(_)) {
        backend.mark_secret_diagnostics_tainted();
    }
    let response_activity = backend.next_activity(request.activity().turn())?;
    let (wire_id, kind) = backend
        .requests
        .get(&request)
        .filter(|binding| !binding.responded)
        .map(|binding| (binding.wire_id.clone(), binding.kind.clone()))
        .ok_or_else(|| protocol::protocol_failure("response has no unanswered Codex request"))?;
    if let RequestKind::Input(questions) = &kind
        && questions.secret_delivery_blocked
    {
        return Err(protocol::protocol_failure(
            "secret input delivery was attempted and this request cannot be retried",
        ));
    }
    let mut next = None;
    let mut response_text;
    let mut answer_seal = None;
    let mut seal_failure = None;
    let navigating = matches!(response, ActivityResponse::PreviousQuestion { .. });
    let (payload, response_kind) = match (&kind, response) {
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
            // 네이티브 기본 메뉴는 레거시 클라이언트의 프로토콜 수준 수락/거부 지원을 제거하지
            // 않습니다. 명시적인 서버 목록이 있을 때만 해당 응답을 제한합니다.
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
        (RequestKind::Input(questions), ActivityResponse::PreviousQuestion { choice, draft }) => {
            if questions.current == 0
                || choice.is_some_and(|choice| {
                    choice == 0
                        || choice as usize > questions.questions[questions.current].options.len()
                })
            {
                return Err(protocol::protocol_failure(
                    "previous question is unavailable or draft choice is invalid",
                ));
            }
            let mut questions = questions.clone();
            let current = questions.current;
            let current_question = questions.questions[current].clone();
            if current_question.is_secret {
                if choice.is_some() || !draft.as_str().is_empty() {
                    return Err(protocol::protocol_failure(
                        "secret question navigation cannot carry an ordinary draft",
                    ));
                }
                questions.discard_secret_value(current);
            } else {
                questions.drafts.insert(
                    current_question.id.clone(),
                    (choice, draft.as_str().to_owned()),
                );
            }
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
            if questions.questions[questions.current].is_secret {
                questions.discard_secret_value(questions.current);
            }
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
            response @ (ActivityResponse::UserInput(_) | ActivityResponse::QuestionAnswer { .. }),
        ) => {
            let mut questions = questions.clone();
            let question = questions.questions[questions.current].clone();
            if question.is_secret {
                return Err(protocol::protocol_failure(
                    "secret questions require the request-bound secret input response",
                ));
            }
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
                ActivityResponse::SecretInput(_) | ActivityResponse::SecretInputSubmitted => {
                    unreachable!("secret response is handled by its dedicated branch")
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
        (RequestKind::Input(questions), ActivityResponse::SecretInput(input)) => {
            let mut questions = questions.clone();
            let question = questions.questions[questions.current].clone();
            if !question.is_secret {
                return Err(protocol::protocol_failure(
                    "secret input response targets a non-secret question",
                ));
            }
            questions.drafts.remove(&question.id);
            let capture_response = input.clone();
            let current = questions.current;
            questions.retain_secret(current, &input)?;
            if let Some(Capture::Batch {
                questions: captured,
                ..
            }) = questions.capture.as_deref()
            {
                let answer = captured[current]
                    .project_response(&ActivityResponse::SecretInput(capture_response))
                    .map_err(|error| protocol::protocol_failure(error.to_string()))?;
                questions.captured_answers[current] = Some((
                    answer,
                    AnswerResponse {
                        question_id: question.id.clone(),
                        request,
                        response_activity,
                    },
                ));
            }
            response_text = questions.receipt("", None);
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
                        match (Capture::AcceptedAnswers {
                            interview: *interview,
                            revision: revision.clone(),
                            answers,
                            answer_responses,
                            final_request: request,
                            response_activity,
                        })
                        .to_snapshot()
                        {
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
    let interview_progress = match &kind {
        RequestKind::Input(questions) => Some((questions.answers.len(), questions.questions.len())),
        RequestKind::Approval { .. } => None,
    };
    let next = next
        .map(|questions| {
            let activity = backend.next_activity(request.activity().turn())?;
            let request_id = backend.next_request()?;
            Ok::<_, BackendFailure>((
                questions,
                ActivityRequestRef::new(activity, request_id),
                events::wire_key(&wire_id)?,
            ))
        })
        .transpose()?;
    if let Some(payload) = payload {
        let secret_batch = matches!(&kind, RequestKind::Input(questions) if questions.has_secret());
        if let Err(failure) = backend.client.respond(wire_id.clone(), payload) {
            if secret_batch {
                if let Some(RequestBinding {
                    kind: RequestKind::Input(questions),
                    ..
                }) = backend.requests.get_mut(&request)
                {
                    questions.discard_secret_values(true);
                }
                return Err(BackendFailure::new(
                    failure.kind(),
                    "secret input delivery failed with an unknown outcome",
                ));
            }
            return Err(if let Some((recorded, total)) = interview_progress {
                BackendFailure::new(
                    failure.kind(),
                    format!(
                        "{}\nInterview incomplete: {recorded}/{total} earlier answers recorded. Final submission was not confirmed.",
                        failure.message()
                    ),
                )
            } else {
                failure
            });
        }
        backend
            .requests
            .get_mut(&request)
            .expect("validated request")
            .responded = true;
        if secret_batch
            && let Some(RequestBinding {
                kind: RequestKind::Input(questions),
                ..
            }) = backend.requests.get_mut(&request)
        {
            questions.discard_secret_values(false);
        }
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
        backend
            .pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity: response_activity,
                kind: response_kind,
            });
        backend
            .pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity: response_activity,
                update: ActivityUpdate::TextSnapshot(response_text),
            });
        backend
            .pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity: response_activity,
                outcome: ActivityOutcome::Completed,
            });
    }
    if let Some((questions, successor, wire_key)) = next {
        let activity = successor.activity();
        let request_id = successor.request_id();
        backend.requests.remove(&request);
        backend.wire_requests.insert(wire_key, successor);
        backend
            .pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity: request.activity(),
                outcome: ActivityOutcome::Completed,
            });
        backend
            .pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            });
        backend
            .pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(questions.prompt()),
            });
        backend.requests.insert(
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
