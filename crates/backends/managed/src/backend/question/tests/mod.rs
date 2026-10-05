use std::sync::{Arc, Mutex};

use yo_backend::BackendAdapter;
use yo_core::{
    AgentCommand, BackendPoll, ModelConnectorEvent, ModelConnectorInputItem,
    ToolApprovalRequirement, UserInput,
};

use super::*;
use crate::backend::{
    NativeModelBackendConfig, NativeModelBackendServices, secret,
    tests::support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, completed,
        context_profile, event_rounds, kimi, registry, turn,
    },
};

fn question_round() -> Vec<ModelConnectorEvent> {
    vec![ModelConnectorEvent::ResponseCreated { response_id: "question".into() },
        ModelConnectorEvent::FunctionCallStarted { output_index: 0, item_id: "item".into(), call_id: "question-call".into(), name: NAME.into() },
        ModelConnectorEvent::FunctionCallDone { output_index: 0, item_id: "item".into(), call_id: "question-call".into(), name: NAME.into(), arguments: r#"{"title":"Choose","question":"Which path?","choices":[{"label":"First","description":"One"}]}"#.into() },
        completed("question")]
}
fn answer_round() -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "answer".into(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: "answer-item".into(),
            content_index: 0,
            delta: "Done".into(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: "answer-item".into(),
        },
        completed("answer"),
    ]
}
type CapturedRequests = Arc<Mutex<Vec<yo_core::ModelConnectorRequest>>>;

fn backend(
    rounds: Vec<Vec<ModelConnectorEvent>>,
) -> (NativeModelBackend, CapturedRequests, Arc<Mutex<usize>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(0));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: requests.clone(),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::with_start_counter(starts.clone())),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig {
            ask_user_enabled: true,
            ..NativeModelBackendConfig::default()
        },
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
            input: UserInput::new("Help"),
        })
        .unwrap();
    (backend, requests, starts)
}
fn question(backend: &mut NativeModelBackend) -> ActivityRequestRef {
    for _ in 0..100 {
        if let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::UserInputRequest { request_id },
        }) = backend.poll_event().unwrap()
        {
            return ActivityRequestRef::new(activity, request_id);
        }
    }
    panic!("question was not opened");
}
fn respond(
    backend: &mut NativeModelBackend,
    request: ActivityRequestRef,
    response: ActivityResponse,
) -> Result<BackendCommandEvidence, BackendFailure> {
    backend.execute_command(AgentCommand::RespondToActivity { request, response })
}

// 첫 초과 UTF-8 바이트, 선택지 개수, 중복 및 null·미지 필드는 질문을 열기 전에 거절한다.
#[test]
fn question_arguments_are_closed_and_byte_bounded() {
    let mut value = json!({"title":"t".repeat(80),"question":"q".repeat(4096),"choices":[{"label":"한".repeat(26),"description":"d".repeat(512)}]});
    assert!(Arguments::parse(&value.to_string()).is_ok());
    for (field, excess) in [("title", "t".repeat(81)), ("question", "q".repeat(4097))] {
        let mut rejected = value.clone();
        rejected[field] = json!(excess);
        assert!(Arguments::parse(&rejected.to_string()).is_err());
    }
    for choices in [
        json!(null),
        json!([{"label":"x","description":""},{"label":"x","description":""}]),
        json!([{"label":"x","description":null}]),
        json!([{"label":"x","description":"","extra":0}]),
        json!([{"label":"x","description":"d".repeat(513)}]),
        json!([{"label":"한".repeat(27),"description":""}]),
    ] {
        value["choices"] = choices;
        assert!(Arguments::parse(&value.to_string()).is_err());
    }
    value["choices"] = json!(
        (0..8)
            .map(|n| json!({"label":n.to_string(),"description":""}))
            .collect::<Vec<_>>()
    );
    assert!(Arguments::parse(&value.to_string()).is_ok());
    value["choices"]
        .as_array_mut()
        .unwrap()
        .push(json!({"label":"9","description":""}));
    assert!(Arguments::parse(&value.to_string()).is_err());
    for raw in [
        r#"{"title":"x","question":"q","other":0}"#,
        r#"{"title":"x\n","question":"q"}"#,
        r#"{"title":"x","question":"\u0000"}"#,
    ] {
        assert!(Arguments::parse(raw).is_err());
    }
    let raw = r#"{"title":"x","question":"q"}"#;
    assert!(Arguments::parse(&format!("{raw}{}", " ".repeat(16_384 - raw.len()))).is_ok());
    assert!(Arguments::parse(&format!("{raw}{}", " ".repeat(16_385 - raw.len()))).is_err());
}

// 입력은 준비 동안 대기 상태를 유지하고 commit 뒤 같은 Turn의 다음 요청에 정확한 결과로 들어간다.
#[test]
fn question_answer_and_unanswered_wait_for_commit_and_preserve_exact_results() {
    for (response, expected) in [
        (
            ActivityResponse::UserInput(UserInput::new("  /exit\n한글  ")),
            json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"text","text":"  /exit\n한글  "}),
        ),
        (
            ActivityResponse::QuestionAnswer {
                choice: 1,
                notes: UserInput::new("  notes\n "),
            },
            json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"choice","choice":1,"label":"First","notes":"  notes\n "}),
        ),
        (
            ActivityResponse::QuestionUnanswered,
            json!({"schema":"yo.ask-user-result/v1","status":"unanswered"}),
        ),
    ] {
        let (mut backend, requests, starts) = backend(vec![question_round(), answer_round()]);
        let request = question(&mut backend);
        assert_eq!(
            respond(&mut backend, request, response).unwrap(),
            BackendCommandEvidence::OrdinaryQuestionResponsePrepared
        );
        for _ in 0..20 {
            backend.poll_event().unwrap();
        }
        assert_eq!(requests.lock().unwrap().len(), 1);
        backend.commit_prepared_command().unwrap();
        for _ in 0..100 {
            backend.poll_event().unwrap();
            if requests.lock().unwrap().len() == 2 {
                break;
            }
        }
        let requests = requests.lock().unwrap();
        let output = requests[1]
            .input()
            .iter()
            .find_map(|item| match item {
                ModelConnectorInputItem::FunctionCallOutput { call_id, output }
                    if call_id == "question-call" =>
                {
                    Some(output)
                },
                _ => None,
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(output).unwrap(),
            expected
        );
        let names = requests[1]
            .tools()
            .unwrap()
            .iter()
            .map(|tool| tool.name())
            .collect::<Vec<_>>();
        assert_eq!(
            &names[names.len() - 2..],
            &["ask_user", "request_secret_input"]
        );
        assert_eq!(*starts.lock().unwrap(), 0);
        assert!(respond(&mut backend, request, ActivityResponse::QuestionUnanswered).is_err());
        backend.shutdown().unwrap();
    }
}

// 부정확한 응답과 첫 초과 답변은 대기 질문을 소비하지 않으며 정확한 최대 답변은 허용한다.
#[test]
fn question_rejections_keep_the_request_available() {
    let (mut backend, _, _) = backend(vec![question_round(), answer_round()]);
    let request = question(&mut backend);
    for response in [
        ActivityResponse::UserInput(UserInput::new("")),
        ActivityResponse::UserInput(UserInput::new("a".repeat(16_385))),
        ActivityResponse::QuestionAnswer {
            choice: 0,
            notes: UserInput::new(""),
        },
        ActivityResponse::QuestionAnswer {
            choice: 2,
            notes: UserInput::new(""),
        },
        ActivityResponse::QuestionAnswer {
            choice: 1,
            notes: UserInput::new("x".repeat(16_385)),
        },
        ActivityResponse::PreviousQuestion {
            choice: None,
            draft: UserInput::new(""),
        },
    ] {
        assert!(respond(&mut backend, request, response).is_err());
        assert!(backend.turn.as_ref().unwrap().awaiting_question.is_some());
    }
    assert!(
        respond(
            &mut backend,
            request,
            ActivityResponse::UserInput(UserInput::new("x".repeat(16_384)))
        )
        .is_ok()
    );
    backend.abort_prepared_command().unwrap();
    assert!(respond(&mut backend, request, ActivityResponse::QuestionUnanswered).is_ok());
    backend.shutdown().unwrap();
}

// 로컬 호출과 질문의 순서가 달라도 완전한 단독 응답이 아니면 질문과 실행 모두 열리지 않는다.
#[test]
fn question_mixed_calls_fail_before_any_question_or_local_effect() {
    for question_first in [true, false] {
        let mut round = question_round();
        let local = vec![
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 1,
                item_id: "local".into(),
                call_id: "local".into(),
                name: "read_file".into(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 1,
                item_id: "local".into(),
                call_id: "local".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a"}"#.into(),
            },
        ];
        let offset = if question_first { 3 } else { 1 };
        round.splice(offset..offset, local);
        let (mut backend, requests, starts) = backend(vec![round]);
        for _ in 0..100 {
            match backend.poll_event() {
                Ok(BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::UserInputRequest { .. },
                    ..
                })) => panic!("mixed response opened a question"),
                Ok(BackendPoll::Event(BackendEvent::TurnFinished { .. })) | Err(_) => break,
                _ => {},
            }
        }
        assert_eq!(*starts.lock().unwrap(), 0);
        assert_eq!(requests.lock().unwrap().len(), 1);
        backend.shutdown().unwrap();
    }
}

// 완성되지 않은 응답은 질문을 열지 않으며 secret과 섞인 요청도 효과를 내지 않는다.
#[test]
fn question_incomplete_and_secret_mixed_rounds_never_open_a_request() {
    let mut incomplete = question_round();
    *incomplete.last_mut().unwrap() = ModelConnectorEvent::Terminal {
        response_id: "question".into(),
        status: yo_core::ModelConnectorTerminal::Incomplete {
            reason: None,
            request_failure: yo_core::ModelRequestFailureKind::ResponseLimit,
        },
        usage: Default::default(),
    };
    let mut mixed = question_round();
    mixed.insert(
        3,
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 1,
            item_id: "secret".into(),
            call_id: "secret".into(),
            name: "request_secret_input".into(),
        },
    );
    for round in [incomplete, mixed] {
        let (mut backend, _, starts) = backend(vec![round]);
        for _ in 0..100 {
            match backend.poll_event() {
                Ok(BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::UserInputRequest { .. },
                    ..
                })) => panic!("invalid response opened a question"),
                Ok(BackendPoll::Event(BackendEvent::TurnFinished { .. })) | Err(_) => break,
                _ => {},
            }
        }
        assert_eq!(*starts.lock().unwrap(), 0);
        backend.shutdown().unwrap();
    }
}

// 응답 replay 용량 검사는 준비 전에 끝나고 실패해도 질문과 원래 call identity가 유지된다.
#[test]
fn question_replay_capacity_failure_does_not_consume_the_response() {
    let (mut backend, _, _) = backend(vec![question_round()]);
    let request = question(&mut backend);
    let saved = backend.turn.as_ref().unwrap().delta.clone();
    backend.turn.as_mut().unwrap().delta = vec![
        ModelReplayItem::Message {
            role: yo_core::ModelReplayRole::Assistant,
            content: "x".into(),
            refusal: None
        };
        yo_core::ModelReplayDelta::MAX_ITEMS
    ];
    assert!(respond(&mut backend, request, ActivityResponse::QuestionUnanswered).is_err());
    assert!(backend.turn.as_ref().unwrap().awaiting_question.is_some());
    assert!(backend.turn.as_ref().unwrap().prepared_question.is_none());
    backend.turn.as_mut().unwrap().delta = saved;
    assert!(respond(&mut backend, request, ActivityResponse::QuestionUnanswered).is_ok());
    backend.shutdown().unwrap();
}

// 새 Session 설정과 무관하게 정확한 네 suffix만 인식하며 질문만 붙은 부분 suffix를 거절한다.
#[test]
fn question_saved_contract_profile_is_exact_and_independent_of_new_session_flag() {
    let (mut backend, _, _) = backend(vec![question_round()]);
    let question_contract = backend.question_contract.clone().unwrap();
    backend.config.ask_user_enabled = false;
    backend.question_enabled = false;
    assert_eq!(
        backend.secret_interaction_profile(&question_contract),
        Some((true, false, true))
    );
    assert_eq!(
        backend.secret_interaction_profile(&backend.current_secret_contract),
        Some((true, false, false))
    );
    assert_eq!(
        backend.secret_interaction_profile(backend.historical_secret_contract.as_ref().unwrap()),
        Some((true, true, false))
    );
    assert_eq!(
        backend.secret_interaction_profile(&backend.legacy_contract),
        Some((false, false, false))
    );
    let mut tools = backend.registry.replay_tools();
    tools.push(replay_tool());
    let partial =
        yo_core::ModelReplayContract::new(backend.config.system_prompt.clone(), tools.clone());
    assert_eq!(backend.secret_interaction_profile(&partial), None);
    tools.push(secret::replay_tool(false));
    let historical = yo_core::ModelReplayContract::new(backend.config.system_prompt.clone(), tools);
    assert_eq!(backend.secret_interaction_profile(&historical), None);
    backend.shutdown().unwrap();
}

// provider private envelope와 질문·출력은 다음 요청과 완료 replay에서 분리되지 않는다.
#[test]
fn question_private_envelope_and_result_stay_in_one_replay_group() {
    let mut round = question_round();
    for event in &mut round {
        match event {
            ModelConnectorEvent::FunctionCallStarted { output_index, .. }
            | ModelConnectorEvent::FunctionCallDone { output_index, .. } => *output_index = 1,
            _ => {},
        }
    }
    round.splice(
        1..1,
        [
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "visible".into(),
                content_index: 0,
                delta: "Choose".into(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "visible".into(),
            },
        ],
    );
    let call = match &round[4] {
        ModelConnectorEvent::FunctionCallDone {
            call_id,
            name,
            arguments,
            ..
        } => ModelReplayItem::FunctionCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        },
        _ => unreachable!(),
    };
    let envelope = kimi::private_envelope("private question reasoning", Some("Choose"));
    round.insert(
        5,
        ModelConnectorEvent::ProviderPrivateAssistant {
            output_index: 2,
            envelope: envelope.clone(),
            visible_projection: vec![
                ModelReplayItem::Message {
                    role: yo_core::ModelReplayRole::Assistant,
                    content: "Choose".into(),
                    refusal: None,
                },
                call,
            ],
        },
    );
    let mut answer = answer_round();
    answer.insert(
        3,
        ModelConnectorEvent::ProviderPrivateAssistant {
            output_index: 1,
            envelope: kimi::private_envelope("private final reasoning", Some("Done")),
            visible_projection: vec![ModelReplayItem::Message {
                role: yo_core::ModelReplayRole::Assistant,
                content: "Done".into(),
                refusal: None,
            }],
        },
    );
    let (mut backend, requests, _) = backend(vec![round, answer]);
    backend.replay_profile = yo_core::ReplayProfile::ProviderPrivateLocalPlaintext;
    let request = question(&mut backend);
    respond(&mut backend, request, ActivityResponse::QuestionUnanswered).unwrap();
    backend.commit_prepared_command().unwrap();
    for _ in 0..100 {
        if matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. })
        ) {
            break;
        }
    }
    let requests = requests.lock().unwrap();
    let input = requests[1].input();
    let private = input.iter().position(|i| matches!(i, ModelConnectorInputItem::ProviderPrivateAssistant { envelope: e } if e == &envelope)).unwrap();
    assert!(
        matches!(&input[private-1], ModelConnectorInputItem::FunctionCall { name, .. } if name == NAME)
    );
    assert!(
        matches!(&input[private+1], ModelConnectorInputItem::FunctionCallOutput { call_id, .. } if call_id == "question-call")
    );
    assert_eq!(backend.replay_groups.len(), 1);
    assert!(backend.replay_groups[0].iter().any(
        |i| matches!(i, ModelReplayItem::ProviderPrivateAssistant { envelope: e } if e == &envelope)
    ));
    assert!(backend.replay_groups[0].iter().any(|i| matches!(i, ModelReplayItem::FunctionCallOutput { call_id, .. } if call_id == "question-call")));
    backend.shutdown().unwrap();
}

struct QuestionPressureCounter;
impl yo_core::ModelTokenCounter for QuestionPressureCounter {
    fn count_input_tokens(
        &self,
        _: &str,
        payload: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        let text = payload.to_string();
        Ok(
            if text.contains("yo.ask-user-result/v1") && !text.contains("# Context Checkpoint") {
                95
            } else {
                10
            },
        )
    }
}

// 질문 응답 뒤 context 압축은 이전 완결 그룹만 요약하고 현재 질문·결과 전체를 그대로 보존한다.
#[test]
fn question_pressure_checkpoint_retains_the_complete_current_group() {
    let summary = [
        "# Context Checkpoint",
        "## Current Objective\nContinue.",
        "## Active Constraints\nNone.",
        "## Decisions\nNone.",
        "## Verified Progress\nPrior turn completed.",
        "## Current State\nQuestion answered.",
        "## Unknown or Unverified\nNone.",
        "## Next Actions\nContinue.",
        "## Critical References\nNone.",
    ]
    .join("\n");
    let mut summary_round = answer_round();
    if let ModelConnectorEvent::TextDelta { delta, .. } = &mut summary_round[1] {
        *delta = summary.clone();
    }
    if let ModelConnectorEvent::Terminal { usage, .. } = summary_round.last_mut().unwrap() {
        usage.input_tokens = Some(10);
        usage.output_tokens = Some(5);
        usage.total_tokens = Some(15);
    }
    let (mut backend, requests, _) = backend(vec![question_round(), summary_round, answer_round()]);
    let request = question(&mut backend);
    let older = vec![
        ModelReplayItem::Message {
            role: yo_core::ModelReplayRole::User,
            content: "old input".into(),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: yo_core::ModelReplayRole::Assistant,
            content: "old answer".into(),
            refusal: None,
        },
    ];
    backend
        .replay
        .apply(&yo_core::ModelReplayDelta::new(
            Some(backend.contract.clone()),
            older.clone(),
        ))
        .unwrap();
    backend.replay_groups.push(older);
    backend.model_context =
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap();
    backend.token_counter = Box::new(QuestionPressureCounter);
    respond(&mut backend, request, ActivityResponse::QuestionUnanswered).unwrap();
    backend.commit_prepared_command().unwrap();
    let mut suffix = None;
    let proposal = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ContextActiveSuffixCompleted { items, .. }) => {
                suffix = Some(items);
                None
            },
            BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { proposal }) => {
                Some(proposal)
            },
            BackendPoll::Event(BackendEvent::TurnFinished { outcome, .. }) => {
                panic!("question checkpoint failed: {outcome:?}")
            },
            _ => None,
        })
        .expect("question boundary must permit pressure compaction");
    assert_eq!(proposal.portable_body(), summary);
    assert_eq!(proposal.summarized_groups().len(), 1);
    let suffix = suffix.unwrap();
    assert!(
        suffix
            .iter()
            .any(|item| matches!(item, ModelReplayItem::FunctionCall { name, .. } if name == NAME))
    );
    assert!(suffix.iter().any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { call_id, .. } if call_id == "question-call")));
    assert_eq!(requests.lock().unwrap().len(), 2);
    backend.poll_event().unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let input = requests[2].input();
    assert!(input.iter().any(
        |item| matches!(item, ModelConnectorInputItem::FunctionCall { name, .. } if name == NAME)
    ));
    assert!(input.iter().any(|item| matches!(item, ModelConnectorInputItem::FunctionCallOutput { call_id, .. } if call_id == "question-call")));
    drop(requests);
    backend.shutdown().unwrap();
}
