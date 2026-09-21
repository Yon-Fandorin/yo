use std::{
    iter,
    sync::{Arc, Mutex},
};

use yo_backend::BackendAdapter;
use yo_core::{
    ActivityKind, ActivityRequestRef, ActivityResponse, ActivityUpdate, AgentCommand,
    BackendCommandEvidence, BackendEvent, BackendPoll, ModelConnectorEvent,
    ModelConnectorInputItem, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    NATIVE_SECRET_INTERACTION_NAME, SecretInput, SecretStorageRecommendation,
    ToolApprovalRequirement, TurnOutcome, UserInput,
};

use super::support::{
    ExactAdmission, FixedTokenCounter, MockConnector, MockHost, completed, context_profile,
    event_rounds, registry, turn,
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

fn secret_round() -> Vec<ModelConnectorEvent> {
    secret_round_with_arguments(
        r#"{"title":"Credential","question":"Enter the token.","purpose":"Authenticate this request."}"#,
    )
}

fn secret_round_with_arguments(arguments: &str) -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "secret-request".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "secret-item".to_owned(),
            call_id: "secret-call".to_owned(),
            name: NATIVE_SECRET_INTERACTION_NAME.to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "secret-item".to_owned(),
            call_id: "secret-call".to_owned(),
            name: NATIVE_SECRET_INTERACTION_NAME.to_owned(),
            arguments: arguments.to_owned(),
        },
        completed("secret-request"),
    ]
}

// 저장 제안은 비밀 질문의 typed 공개 snapshot으로 전달되고 모델 요청의 스키마에도 나타난다.
#[test]
fn native_secret_request_exposes_typed_storage_offer_without_granting_storage() {
    let arguments = serde_json::json!({
        "title": "Credential",
        "question": "Enter the token.",
        "purpose": "Authenticate this request.",
        "storage_offer": {
            "scope": "github.token",
            "recommendation": "store_for_days",
            "reason": "Reuse this token for later requests.\nProvider: spoof",
            "suggested_days": 30
        }
    })
    .to_string();
    let (mut backend, requests) = started_backend_with_rounds(
        "use the credential".to_owned(),
        vec![
            secret_round_with_arguments(&arguments),
            answer_round("done"),
        ],
    );
    let (_request, presentation, _) = poll_secret_request(&mut backend);
    let question = yo_core::ActivityQuestion::from_snapshot(&presentation).unwrap();
    let offer = question.storage_offer.unwrap();
    assert_eq!(offer.scope, "github.token");
    assert_eq!(
        offer.recommendation,
        SecretStorageRecommendation::StoreForDays
    );
    assert_eq!(
        offer.reason,
        "Reuse this token for later requests.\nProvider: spoof"
    );
    assert_eq!(offer.suggested_days, Some(30));
    assert!(question.plain_text.contains("Provider: qwencloud"));
    assert!(question.plain_text.contains("Model: qwen3.8max"));
    assert!(question.plain_text.contains("Use once (default)"));
    assert!(question.plain_text.contains("Store for… (1–365 days)"));
    assert!(question.plain_text.contains("Store until deleted"));
    assert!(
        question
            .plain_text
            .contains("Ctrl-S to choose locally; Enter separately submits")
    );
    assert!(
        question
            .plain_text
            .contains("Storage requires a verified destination account.")
    );
    assert!(
        question
            .plain_text
            .contains("Reason: Reuse this token for later requests.\\nProvider: spoof")
    );
    assert!(!question.plain_text.contains("\nProvider: spoof"));
    let tools = requests.lock().unwrap();
    assert!(
        tools[0].tools().unwrap().last().unwrap().parameters()["properties"]
            .get("storage_offer")
            .is_some()
    );
}

fn answer_round(answer: &str) -> Vec<ModelConnectorEvent> {
    answer_round_with_id("secret-answer", answer)
}

fn answer_round_with_id(response_id: &str, answer: &str) -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: response_id.to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: "answer-item".to_owned(),
            content_index: 0,
            delta: answer.to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: "answer-item".to_owned(),
        },
        completed(response_id),
    ]
}

fn malformed_answer_round() -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "secret-answer".to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: "answer-item-0".to_owned(),
            content_index: 0,
            delta: "first".to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: "answer-item-0".to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 1,
            item_id: "answer-item-1".to_owned(),
            content_index: 0,
            delta: "second".to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 1,
            item_id: "answer-item-1".to_owned(),
        },
        completed("secret-answer"),
    ]
}

fn started_backend(
    answer: &str,
) -> (
    NativeModelBackend,
    Arc<Mutex<Vec<yo_core::ModelConnectorRequest>>>,
) {
    started_backend_with_rounds(
        "use the credential".to_owned(),
        vec![secret_round(), answer_round(answer)],
    )
}

fn started_backend_with_rounds(
    input: String,
    rounds: Vec<Vec<ModelConnectorEvent>>,
) -> (
    NativeModelBackend,
    Arc<Mutex<Vec<yo_core::ModelConnectorRequest>>>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::clone(&requests),
        }),
        super::support::binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from(input),
        })
        .unwrap();
    (backend, requests)
}

fn poll_secret_request(
    backend: &mut NativeModelBackend,
) -> (ActivityRequestRef, String, Vec<serde_json::Value>) {
    let mut request = None;
    let mut usage_receipts = Vec::new();
    for _ in 0..100 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            }) => request = Some(ActivityRequestRef::new(activity, request_id)),
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) if request.is_some() => return (request.unwrap(), text, usage_receipts),
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) => {
                if let Ok(receipt) = serde_json::from_str::<serde_json::Value>(&text)
                    && receipt["schema"] == "yo.model-usage-receipt/v1"
                {
                    usage_receipts.push(receipt);
                }
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the secret request"),
        }
    }
    panic!("secret request was not presented")
}

// 비밀 요청의 대상·경고를 먼저 표시하고 영수증 commit 뒤에만 최종 요청을 한 번 시작하는지 확인한다.
#[test]
fn native_secret_request_waits_for_commit_and_finishes_without_replay() {
    let (mut backend, requests) = started_backend("done");
    let (request, presentation, mut usage_receipts) = poll_secret_request(&mut backend);
    assert!(presentation.contains("Provider: qwencloud"));
    assert!(presentation.contains("Model: qwen3.8max"));
    assert!(presentation.contains("They may retain it."));
    assert!(presentation.contains("makes this Session unavailable"));
    assert!(presentation.contains("exact secret-echo check"));
    let first = requests.lock().unwrap();
    let tools = first[0].tools().unwrap();
    assert_eq!(tools.last().unwrap().name(), NATIVE_SECRET_INTERACTION_NAME);
    drop(first);

    let evidence = backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("canary-value").unwrap()),
        })
        .unwrap();
    assert_eq!(evidence, BackendCommandEvidence::ProtectedInputPrepared);
    assert_eq!(requests.lock().unwrap().len(), 1);

    backend.commit_prepared_command().unwrap();
    let mut visible = String::new();
    let terminal = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) if text == "done" => visible = text,
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) => {
                if let Ok(receipt) = serde_json::from_str::<serde_json::Value>(&text)
                    && receipt["schema"] == "yo.model-usage-receipt/v1"
                {
                    usage_receipts.push(receipt);
                }
            },
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => break event,
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                panic!("secret terminal Turn must not be resumable")
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before terminal completion"),
        }
    };
    assert_eq!(visible, "done");
    assert!(matches!(
        terminal,
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));
    assert_eq!(usage_receipts.len(), 2);
    assert_eq!(usage_receipts[0]["response_id"], "secret-request");
    assert_eq!(usage_receipts[0]["round"], 1);
    assert_eq!(usage_receipts[1]["response_id"], "secret-answer");
    assert_eq!(usage_receipts[1]["round"], 2);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].tools().is_none());
    assert!(requests[1].has_protected_terminal_input());
    assert!(matches!(
        requests[1].input().last(),
        Some(ModelConnectorInputItem::FunctionCallOutput { call_id, output })
            if call_id == "secret-call" && output == "canary-value"
    ));
}

// 최종 답변에 입력한 비밀 바이트열이 있으면 어떤 모델 텍스트도 공개하지 않는지 확인한다.
#[test]
fn exact_secret_echo_withholds_the_whole_answer() {
    let (mut backend, requests) = started_backend("prefix canary-value suffix");
    let (request, _, _) = poll_secret_request(&mut backend);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("canary-value").unwrap()),
        })
        .unwrap();
    backend.commit_prepared_command().unwrap();
    let mut answer_published = false;
    let terminal = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                kind: ActivityKind::AgentMessage,
                ..
            }) => answer_published = true,
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => break event,
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before terminal failure"),
        }
    };
    assert!(!answer_published);
    assert!(matches!(
        terminal,
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
    assert_eq!(requests.lock().unwrap().len(), 2);
}

// Provider 응답 ID가 비밀을 반복해도 usage 영수증이나 보이는 답변으로 남지 않는지 확인한다.
#[test]
fn exact_secret_echo_in_response_id_withholds_all_provider_output() {
    let (mut backend, _) = started_backend_with_rounds(
        "use the credential".to_owned(),
        vec![
            secret_round(),
            answer_round_with_id("answer-canary-value", "done"),
        ],
    );
    let (request, _, mut usage_receipts) = poll_secret_request(&mut backend);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("canary-value").unwrap()),
        })
        .unwrap();
    backend.commit_prepared_command().unwrap();

    let mut snapshots = Vec::new();
    let terminal = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) => {
                if let Ok(receipt) = serde_json::from_str::<serde_json::Value>(&text)
                    && receipt["schema"] == "yo.model-usage-receipt/v1"
                {
                    usage_receipts.push(receipt);
                }
                snapshots.push(text);
            },
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => break event,
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before terminal failure"),
        }
    };

    assert!(matches!(
        terminal,
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
    assert_eq!(usage_receipts.len(), 1);
    assert!(
        snapshots
            .iter()
            .all(|snapshot| !snapshot.contains("canary-value"))
    );
    assert!(snapshots.iter().all(|snapshot| snapshot != "done"));
}

// 안전한 응답 ID의 completed 사용량은 최종 메시지 모양이 잘못돼도 한 번 기록되는지 확인한다.
#[test]
fn malformed_terminal_secret_answer_retains_safe_usage_receipt() {
    let (mut backend, _) = started_backend_with_rounds(
        "use the credential".to_owned(),
        vec![secret_round(), malformed_answer_round()],
    );
    let (request, _, mut usage_receipts) = poll_secret_request(&mut backend);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("canary-value").unwrap()),
        })
        .unwrap();
    backend.commit_prepared_command().unwrap();

    let terminal = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) => {
                if let Ok(receipt) = serde_json::from_str::<serde_json::Value>(&text)
                    && receipt["schema"] == "yo.model-usage-receipt/v1"
                {
                    usage_receipts.push(receipt);
                }
            },
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => break event,
            BackendPoll::Event(BackendEvent::ActivityStarted {
                kind: ActivityKind::AgentMessage,
                ..
            }) => panic!("malformed terminal output must not publish an answer"),
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before terminal failure"),
        }
    };

    assert!(matches!(
        terminal,
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
    assert_eq!(usage_receipts.len(), 2);
    assert_eq!(usage_receipts[1]["response_id"], "secret-answer");
    assert_eq!(usage_receipts[1]["round"], 2);
}

// 재생하지 않는 최종 답변은 기존 Turn delta와 합친 replay 한도를 넘겨도 개별 경계 안이면 공개한다.
#[test]
fn terminal_secret_answer_ignores_irrelevant_replay_delta_capacity() {
    let input = "i".repeat(ModelReplayDelta::MAX_ENCODED_BYTES - 2 * 1024 * 1024);
    let answer = "a".repeat(3 * 1024 * 1024);
    let (mut backend, _) =
        started_backend_with_rounds(input, vec![secret_round(), answer_round(&answer)]);
    let (request, _, _) = poll_secret_request(&mut backend);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("canary-value").unwrap()),
        })
        .unwrap();

    let terminal_item = ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: answer.clone(),
        refusal: None,
    };
    let state = backend
        .turn
        .as_ref()
        .expect("secret request remains active");
    let contract = backend
        .replay
        .contract()
        .is_none()
        .then_some(&backend.contract);
    let prior_bytes = ModelReplayDelta::prospective_encoded_len(contract, state.delta.iter())
        .expect("the prior delta has a bounded item count");
    let combined_bytes = ModelReplayDelta::prospective_encoded_len(
        contract,
        state.delta.iter().chain(iter::once(&terminal_item)),
    )
    .expect("the combined delta has a bounded item count");
    assert!(prior_bytes <= ModelReplayDelta::MAX_ENCODED_BYTES);
    assert!(combined_bytes > ModelReplayDelta::MAX_ENCODED_BYTES);

    backend.commit_prepared_command().unwrap();
    let mut visible = false;
    let terminal = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) if text == answer => visible = true,
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => break event,
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before terminal completion"),
        }
    };
    assert!(visible);
    assert!(matches!(
        terminal,
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));
}

// 비밀 요청과 일반 도구 호출이 한 응답에 섞이면 숨김 입력을 열기 전에 전체 Turn을 거부한다.
#[test]
fn native_secret_request_rejects_a_mixed_function_call_response() {
    let mut mixed = secret_round();
    mixed.insert(
        3,
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 1,
            item_id: "ordinary-item".to_owned(),
            call_id: "ordinary-call".to_owned(),
            name: "read_file".to_owned(),
        },
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![mixed]),
            requests: Arc::clone(&requests),
        }),
        super::support::binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("use the credential"),
        })
        .unwrap();

    let terminal = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => break event,
            BackendPoll::Event(BackendEvent::ActivityStarted {
                kind: ActivityKind::UserInputRequest { .. },
                ..
            }) => panic!("mixed function calls must not open a secret editor"),
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before rejecting mixed calls"),
        }
    };
    assert!(matches!(
        terminal,
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
}
