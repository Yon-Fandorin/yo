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
    let response_activity = backend.next_activity(request.activity().turn())?;
    let binding = backend
        .requests
        .get(&request)
        .filter(|binding| !binding.responded)
        .ok_or_else(|| protocol::protocol_failure("response has no unanswered Codex request"))?;
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
            response @ (ActivityResponse::UserInput(_) | ActivityResponse::QuestionAnswer { .. }),
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
        backend.client.respond(wire_id.clone(), payload).map_err(|failure| {
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
        backend
            .requests
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
