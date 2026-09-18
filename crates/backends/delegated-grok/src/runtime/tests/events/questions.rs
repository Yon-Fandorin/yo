use yo_core::{ActivityQuestion, ActivityRequestRef, ActivityUpdate, interview::Capture};

use super::super::*;

fn two_questions() -> Value {
    json!([
        {
            "question": "Which database?",
            "options": [
                {
                    "label": "Postgres",
                    "description": "Relational database",
                    "preview": "CREATE EXTENSION vector;"
                },
                { "label": "SQLite", "description": "Embedded database" }
            ],
            "multiSelect": false
        },
        {
            "question": "Which cache?",
            "options": [
                { "label": "Redis", "description": "Shared cache" },
                { "label": "None", "description": "Do not add a cache" }
            ]
        }
    ])
}

fn one_question() -> Value {
    json!([{
        "question": "Which database?",
        "options": [
            { "label": "Postgres", "description": "Relational database" },
            { "label": "SQLite", "description": "Embedded database" }
        ],
        "multiSelect": false
    }])
}

fn start_question_turn(
    messages: impl IntoIterator<Item = Value>,
) -> (Backend<FakePeer>, Sent, TurnRef) {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let mut later = messages.into_iter().collect::<Vec<_>>();
    later.push(response(4, json!({ "stopReason": "end_turn" })));
    let (mut backend, sent) = backend(
        [
            vec![response(3, json!({ "sessionId": "grok-session-a" }))],
            later,
        ]
        .concat(),
    );
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("ask me"),
        })
        .unwrap();
    (backend, sent, active_turn)
}

fn expect_question(backend: &mut Backend<FakePeer>) -> ActivityRequestRef {
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::UserInputRequest { request_id },
    }) = backend.poll_event().unwrap()
    else {
        panic!("expected Grok user-input request");
    };
    ActivityRequestRef::new(activity, request_id)
}

// 질문 RPC가 prompt의 첫 메시지이고 뒤에 완료 응답이 없어도 그 요청 자체를 수락 증거로
// 사용해 StartTurn을 반환하며, 대기 중인 질문을 즉시 공개합니다.
#[test]
fn first_question_request_admits_the_prompt_without_a_later_response() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        response(3, json!({ "sessionId": "grok-session-a" })),
        question_request("question-a", one_question()),
    ]);
    create_session(&mut backend, session_id);

    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("ask me"),
            })
            .unwrap(),
        BackendCommandEvidence::RequestAccepted(_)
    ));
    let request = expect_question(&mut backend);
    assert_eq!(request.activity().turn(), active_turn);
    expect_activity_update(&mut backend, request.activity());
}

// 같은 Turn에서 하나의 toolCallId로 질문 RPC를 두 번 보내면 첫 질문의 결합을 유지하고
// 두 번째 wire request를 cancelled로 해제한 뒤 Protocol 실패로 닫습니다.
#[test]
fn rejects_duplicate_question_tool_call_while_the_first_request_is_pending() {
    let (mut backend, sent, _) = start_question_turn([
        question_request("question-a", one_question()),
        question_request("question-b", one_question()),
    ]);
    let first = expect_question(&mut backend);
    expect_activity_update(&mut backend, first.activity());

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("duplicate or stale"));
    assert_eq!(backend.inputs.len(), 1);
    assert_eq!(backend.inputs[&first].tool_call_id, "question-tool-a");
    assert!(sent.0.borrow().iter().any(|message| {
        message.get("id") == Some(&json!("question-b"))
            && message["result"]["outcome"] == "cancelled"
    }));
}

// 완료된 이전 Turn의 질문 toolCallId는 Session tombstone으로 남겨 다음 Turn에 늦게 도착한
// 요청이 현재 Turn의 사용자 입력으로 다시 결합되지 않게 합니다.
#[test]
fn rejects_question_tool_call_reused_by_a_later_turn() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let second_turn = turn(session_id, 2);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        question_request_for_tool("question-a", "shared-tool", one_question()),
        response(4, json!({ "stopReason": "end_turn" })),
        question_request_for_tool("question-b", "shared-tool", one_question()),
    ];
    let (mut backend, sent) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("first"),
        })
        .unwrap();
    let request = expect_question(&mut backend);
    expect_activity_update(&mut backend, request.activity());
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::QuestionAnswer {
                choice: 1,
                notes: UserInput::from(""),
            },
        })
        .unwrap();
    for _ in 0..4 {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(_)
        ));
    }
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) if turn == first_turn
    ));

    backend
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second"),
        })
        .unwrap();
    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("duplicate or stale"));
    assert!(sent.0.borrow().iter().any(|message| {
        message.get("id") == Some(&json!("question-b"))
            && message["result"]["outcome"] == "cancelled"
    }));
}

// Session에 보존하는 질문 ToolCallId tombstone이 정확히 상한에 닿은 뒤의 첫 새 ID는
// 상태를 더 늘리지 않고 wire request를 cancelled로 해제합니다.
#[test]
fn bounds_the_session_question_tool_call_tombstones() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend([
        response(3, json!({ "sessionId": "grok-session-a" })),
        question_request("question-overflow", one_question()),
    ]);
    create_session(&mut backend, session_id);
    backend.input_tool_turns.extend(
        (0..Backend::<FakePeer>::MAX_SESSION_TOOL_IDS)
            .map(|index| (format!("question-tool-{index}"), active_turn)),
    );
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("ask me"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("ToolCallId limit"));
    assert_eq!(
        backend.input_tool_turns.len(),
        Backend::<FakePeer>::MAX_SESSION_TOOL_IDS
    );
    assert!(sent.0.borrow().iter().any(|message| {
        message.get("id") == Some(&json!("question-overflow"))
            && message["result"]["outcome"] == "cancelled"
    }));
}

fn drain_answer_and_next_question(
    backend: &mut Backend<FakePeer>,
    answered: ActivityRequestRef,
) -> ActivityRequestRef {
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted {
            kind: ActivityKind::UserInputResponse { request_id },
            ..
        }) if request_id == answered.request_id()
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            outcome: ActivityOutcome::Completed,
            ..
        })
    ));
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity: answered.activity(),
            outcome: ActivityOutcome::Completed,
        })
    );
    expect_question(backend)
}

// Grok 전용 질문 RPC를 공통 interview로 투영하고, 중간 답은 로컬에만 보존한 뒤 마지막
// 답에서 원래 wire request에 전체 accepted payload를 한 번만 반환합니다.
#[test]
fn maps_question_batch_and_returns_one_accepted_response() {
    let (mut backend, sent, active_turn) =
        start_question_turn([question_request("question-a", two_questions())]);

    let first = expect_question(&mut backend);
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextSnapshot(snapshot),
    }) = backend.poll_event().unwrap()
    else {
        panic!("expected complete interview capture");
    };
    assert_eq!(activity, first.activity());
    assert!(matches!(
        Capture::from_snapshot(&snapshot).unwrap(),
        Capture::Batch { questions, .. }
            if questions.len() == 2
                && questions[0].question == "Which database?"
                && questions[1].question == "Which cache?"
    ));

    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: first,
            response: ActivityResponse::QuestionAnswer {
                choice: 1,
                notes: UserInput::from("Use extensions"),
            },
        })
        .unwrap();
    let second = drain_answer_and_next_question(&mut backend, first);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(snapshot),
        }) if activity == second.activity()
            && matches!(Capture::from_snapshot(&snapshot).unwrap(), Capture::Question { question, .. } if question.question == "Which cache?")
    ));

    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: second,
            response: ActivityResponse::UserInput(UserInput::from("Local in-memory cache")),
        })
        .unwrap();

    let response = sent
        .0
        .borrow()
        .iter()
        .find(|message| message.get("id") == Some(&json!("question-a")))
        .cloned()
        .expect("question response");
    assert_eq!(response["result"]["outcome"], "accepted");
    assert_eq!(
        response["result"]["answers"]["Which database?"],
        json!(["Postgres"])
    );
    assert_eq!(
        response["result"]["annotations"]["Which database?"]["notes"],
        "Use extensions"
    );
    assert_eq!(
        response["result"]["annotations"]["Which database?"]["preview"],
        "CREATE EXTENSION vector;"
    );
    assert_eq!(
        response["result"]["answers"]["Which cache?"],
        json!(["Other"])
    );
    assert_eq!(
        response["result"]["annotations"]["Which cache?"]["notes"],
        "Local in-memory cache"
    );
    assert!(backend.inputs.is_empty());

    let BackendPoll::Event(BackendEvent::ActivityStarted {
        kind: ActivityKind::UserInputResponse { request_id },
        ..
    }) = backend.poll_event().unwrap()
    else {
        panic!("expected final answer response activity");
    };
    assert_eq!(request_id, second.request_id());
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(snapshot),
        ..
    }) = backend.poll_event().unwrap()
    else {
        panic!("expected final answer seal");
    };
    assert!(matches!(
        Capture::from_snapshot(&snapshot).unwrap(),
        Capture::AcceptedAnswers { answers, .. } if answers.len() == 2
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) if turn == active_turn
    ));
}

// Shift+Tab에 해당하는 PreviousQuestion은 Grok RPC를 끝내지 않고 현재 draft를 보존해
// 이전 질문으로 이동하며, 다시 전진했을 때 같은 batch의 두 번째 질문을 복원합니다.
#[test]
fn navigates_back_without_answering_the_wire_request() {
    let (mut backend, sent, _) =
        start_question_turn([question_request("question-a", two_questions())]);
    let first = expect_question(&mut backend);
    expect_activity_update(&mut backend, first.activity());
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: first,
            response: ActivityResponse::QuestionAnswer {
                choice: 1,
                notes: UserInput::from("  first note  "),
            },
        })
        .unwrap();
    let second = drain_answer_and_next_question(&mut backend, first);
    expect_activity_update(&mut backend, second.activity());

    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: second,
            response: ActivityResponse::PreviousQuestion {
                choice: Some(2),
                draft: UserInput::from("second draft"),
            },
        })
        .unwrap();
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity: second.activity(),
            outcome: ActivityOutcome::Completed,
        })
    );
    let first_again = expect_question(&mut backend);
    expect_activity_update(&mut backend, first_again.activity());
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextSnapshot(snapshot),
    }) = backend.poll_event().unwrap()
    else {
        panic!("expected restored Grok question draft");
    };
    let restored = ActivityQuestion::from_snapshot(&snapshot).expect("restored question profile");
    assert_eq!(activity, first_again.activity());
    assert_eq!(restored.draft_choice, Some(1));
    assert_eq!(restored.draft.as_deref(), Some("  first note  "));
    let binding = backend.inputs.get(&first_again).unwrap();
    assert_eq!(binding.questions.current, 0);
    assert_eq!(binding.questions.drafts[0].0, Some(1));
    assert_eq!(binding.questions.drafts[0].1, "  first note  ");
    assert_eq!(binding.questions.drafts[1].0, Some(2));
    assert_eq!(binding.questions.drafts[1].1, "second draft");
    assert!(
        sent.0
            .borrow()
            .iter()
            .all(|message| message.get("id") != Some(&json!("question-a")))
    );
}

// 현재 공통 interview가 표현하지 못하는 다중 선택을 단일 선택으로 축소하지 않고,
// Grok tool을 typed cancelled로 해제한 뒤 명시적인 Unsupported를 반환합니다.
#[test]
fn rejects_multi_select_without_losing_a_selection() {
    let questions = json!([{
        "question": "Choose targets",
        "options": [
            { "label": "macOS", "description": "Apple host" },
            { "label": "Linux", "description": "Linux host" }
        ],
        "multiSelect": true
    }]);
    let (mut backend, sent, _) = start_question_turn([question_request("question-a", questions)]);

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Unsupported);
    assert!(failure.message().contains("multi-select"));
    assert!(backend.inputs.is_empty());
    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({
            "jsonrpc": "2.0",
            "id": "question-a",
            "result": { "outcome": "cancelled" }
        })
    );
}

// Turn interrupt는 대기 중인 Grok questionnaire를 typed cancelled로 끝내고, 동일한
// Activity를 Interrupted로 닫아 child의 blocking oneshot과 Yo 상태를 함께 해제합니다.
#[test]
fn interrupt_cancels_the_pending_question_request() {
    let (mut backend, sent, active_turn) =
        start_question_turn([question_request("question-a", two_questions())]);
    let request = expect_question(&mut backend);
    expect_activity_update(&mut backend, request.activity());

    backend
        .execute_command(AgentCommand::InterruptTurn { turn: active_turn })
        .unwrap();

    assert!(backend.inputs.is_empty());
    assert!(sent.0.borrow().iter().any(|message| {
        message.get("id") == Some(&json!("question-a"))
            && message["result"]["outcome"] == "cancelled"
    }));
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Interrupted,
        })
    );
}

// 외부 리뷰 profile은 새 상호작용으로 권한을 넓히지 않고 질문 RPC를 취소한 뒤
// 기존 fail-closed review delivery 경계를 유지합니다.
#[test]
fn read_only_review_rejects_user_questions() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend_with_profile(
        [
            response(3, json!({ "sessionId": "grok-session-a" })),
            question_request("question-a", two_questions()),
            response(4, json!({ "stopReason": "end_turn" })),
        ],
        true,
    );
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("review"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("read-only delegated review"));
    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({
            "jsonrpc": "2.0",
            "id": "question-a",
            "result": { "outcome": "cancelled" }
        })
    );
}

// 완전한 typed presentation을 만들 수 없는 oversized 질문은 평문으로 축약해 받지 않고
// wire invalid-params와 Protocol 실패로 닫습니다.
#[test]
fn rejects_question_that_exceeds_the_display_limit() {
    let questions = json!([{
        "question": "x".repeat(yo_core::ToolOutput::MAX_SNAPSHOT_BYTES),
        "options": []
    }]);
    let (mut backend, sent, _) = start_question_turn([question_request("question-a", questions)]);

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("display limit"));
    assert_eq!(sent.0.borrow().last().unwrap()["error"]["code"], -32602);
}

// 지원 필드의 잘못된 wire 타입은 기본값으로 흡수하지 않고 JSON-RPC invalid-params로
// 응답해 child의 대기 요청과 Yo의 Turn을 함께 실패시킵니다.
#[test]
fn rejects_malformed_question_request_with_invalid_params() {
    let mut request = question_request("question-a", two_questions());
    request["params"]["questions"][0]["multiSelect"] = json!("yes");
    let (mut backend, sent, _) = start_question_turn([request]);

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("must be boolean"));
    assert_eq!(sent.0.borrow().last().unwrap()["error"]["code"], -32602);
    assert!(backend.inputs.is_empty());
}

// Agent가 질문 응답을 받기 전에 prompt 완료를 보내면 Turn 완료로 오인하지 않고
// unresolved-input protocol 실패를 반환합니다.
#[test]
fn rejects_prompt_completion_with_an_unresolved_question() {
    let (mut backend, _, _) =
        start_question_turn([question_request("question-a", two_questions())]);
    let request = expect_question(&mut backend);
    expect_activity_update(&mut backend, request.activity());

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(
        failure
            .message()
            .contains("unresolved user question request")
    );
}

// 노출된 선택 profile은 Grok option 순서와 설명을 그대로 유지하고 비밀 입력으로
// 재분류하지 않습니다.
#[test]
fn exposes_single_select_choices_as_a_public_question() {
    let (mut backend, _, _) =
        start_question_turn([question_request("question-a", two_questions())]);
    let request = expect_question(&mut backend);
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(snapshot),
        ..
    }) = backend.poll_event().unwrap()
    else {
        panic!("expected question capture");
    };
    let Capture::Batch { questions, .. } = Capture::from_snapshot(&snapshot).unwrap() else {
        panic!("expected batch capture");
    };
    let profile = questions[0].presentation(0, questions.len());
    assert_eq!(request.activity().turn(), turn(session(1), 1));
    assert!(profile.plain_text.contains("Which database?"));
    assert!(!profile.is_secret);
    assert!(profile.allow_notes);
    assert_eq!(
        profile.choices,
        vec![
            yo_core::QuestionChoice {
                label: "Postgres".into(),
                description: "Relational database\n\nPreview:\nCREATE EXTENSION vector;".into(),
            },
            yo_core::QuestionChoice {
                label: "SQLite".into(),
                description: "Embedded database".into(),
            }
        ]
    );
    assert!(ActivityQuestion::from_snapshot(&profile.to_snapshot().unwrap()).is_some());
}
