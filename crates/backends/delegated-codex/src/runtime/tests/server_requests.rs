use serde_json::{Value, json};
use yo_core::{
    ActivityApproval, ActivityKind, ActivityOutcome, ActivityQuestion, ActivityRequestRef,
    ActivityResponse, ActivityUpdate, AgentCommand, AgentEvent, AgentRuntime, ApprovalDecision,
    BackendFailureKind, RequestId, RuntimePoll, UserInput,
};

use super::support::{activity, backend, id, session, submission, thread_start_response, turn};

// server approval request를 상관관계 Activity로 노출하고 사용자 결정을 원래 JSON-RPC
// id에 응답한 뒤 request와 response Activity를 각각 완료하는지 확인한다.
#[test]
fn correlates_an_approval_round_trip() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "id": "approval-a",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "itemId": "item-a",
                "command": "cargo test",
                "reason": "requires workspace execution"
            }
        }),
        json!({
            "method": "serverRequest/resolved",
            "params": { "threadId": "thread-a", "requestId": "approval-a" }
        }),
    ];
    let (backend, sent) = backend(messages);
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("inspect"),
            },
            submission(2),
        )
        .unwrap();
    let request = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));

    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: request.activity(),
            kind: ActivityKind::ApprovalRequest {
                request_id: request.request_id(),
            },
        })
    );
    let RuntimePoll::Event(AgentEvent::ActivityUpdated {
        activity: observed_activity,
        update: ActivityUpdate::TextSnapshot(snapshot),
    }) = runtime.poll_event().unwrap()
    else {
        panic!("approval profile missing")
    };
    assert_eq!(observed_activity, request.activity());
    let profile = ActivityApproval::from_snapshot(&snapshot).unwrap();
    assert_eq!(
        profile.plain_text,
        "Command approval\nCommand: cargo test\nReason: requires workspace execution"
    );
    assert_eq!(profile.decline_choice, Some(2));
    assert_eq!(profile.choices[1].label, "Decline and stop");
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();

    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({ "id": "approval-a", "result": { "decision": "accept" } })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: activity(active_turn, 2),
            kind: ActivityKind::ApprovalResponse {
                request_id: request.request_id(),
            },
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: activity(active_turn, 2),
            update: ActivityUpdate::TextSnapshot("Decision: approved".to_owned()),
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: activity(active_turn, 2),
            outcome: ActivityOutcome::Completed,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        })
    );
}

// 지원하지 않는 server request를 받으면 조용히 버려 Codex를 영원히 기다리게 하지 않고,
// 같은 JSON-RPC id에 Method not found 오류를 답한 뒤 Unsupported로 보고하는지 확인한다.
#[test]
fn unsupported_server_request_is_rejected_on_the_wire() {
    let (mut backend, sent) = backend([json!({
        "id": "request-a",
        "method": "item/permissions/requestApproval",
        "params": {}
    })]);

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Unsupported);
    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({
            "id": "request-a",
            "error": {
                "code": -32601,
                "message": "server request is unsupported by yo"
            }
        })
    );
}

// 여러 질문을 순서대로 표시하고 선택 번호·직접 답변을 원래 질문 ID에 묶어 한 번만 응답한다.
#[test]
fn answers_codex_questions_sequentially_with_exact_wire_ids() {
    use yo_core::ActivityUpdate;
    for notes in [None, Some(""), Some("  Keep 한글\nand 2 literal  ")] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"questions-a","method":"item/tool/requestUserInput","params":{
                "threadId":"thread-a","turnId":"turn-a","itemId":"ask","isBlocking":true,
                "questions":[
                    {"id":"scope","header":"Scope","question":"Which area?","options":[{"label":"UI","description":"Layout"},{"label":"Runtime","description":"Events"}]},
                    {"id":"detail","header":"Details","question":"What matters?","options":null}
                ]
            }}),
            json!({"method":"serverRequest/resolved","params":{"threadId":"thread-a","requestId":"questions-a"}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: UserInput::from("ask"),
                },
                submission(1),
            )
            .unwrap();
        let first = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                kind: ActivityKind::UserInputRequest { .. },
                ..
            })
        ));
        let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(prompt),
            ..
        }) = runtime.poll_event().unwrap()
        else {
            panic!("question prompt")
        };
        let question = ActivityQuestion::from_snapshot(&prompt).unwrap();
        assert_eq!(question.choices[1].label, "Runtime");
        assert_eq!(question.choices[1].description, "Events");
        assert!(question.plain_text.contains(
            "This answer is recorded locally. All answers are sent after the final question."
        ));
        assert!(prompt.contains("Question 1 of 2"));
        assert!(prompt.contains("2. Runtime"));
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request: first,
                response: match notes {
                    None => ActivityResponse::UserInput(UserInput::from("2")),
                    Some(notes) => ActivityResponse::QuestionAnswer {
                        choice: 2,
                        notes: UserInput::from(notes),
                    },
                },
            })
            .unwrap();
        assert!(
            !sent
                .0
                .borrow()
                .iter()
                .any(|message| message.get("id") == Some(&json!("questions-a")))
        );
        let mut continuation = Vec::new();
        for _ in 0..6 {
            continuation.push(runtime.poll_event().unwrap());
        }
        let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: receipt_activity,
            update: ActivityUpdate::TextSnapshot(receipt),
        }) = &continuation[1]
        else {
            panic!("answer receipt missing")
        };
        assert_eq!(*receipt_activity, activity(active_turn, 2));
        assert!(receipt.contains("Question 1 of 2\nWhich area?\n\nAnswer: Runtime"));
        assert!(receipt.contains("Recorded; waiting for the remaining questions."));
        assert!(!receipt.contains("responses sent"));
        if let Some(notes) = notes.filter(|notes| !notes.is_empty()) {
            assert!(receipt.contains(&format!("Note: {}", notes.trim())));
        } else {
            assert!(!receipt.contains("Note:"));
        }
        let second = ActivityRequestRef::new(activity(active_turn, 3), RequestId::new(id(2)));
        assert!(
            matches!(&continuation[5],RuntimePoll::Event(AgentEvent::ActivityUpdated{activity,update:ActivityUpdate::TextSnapshot(text)}) if *activity==second.activity() && text.contains("Question 2 of 2") && ActivityQuestion::from_snapshot(text).unwrap().plain_text.contains("Submitting this answer sends all 2 answers."))
        );
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request: second,
                response: ActivityResponse::UserInput(UserInput::from("Keep 한글 intact")),
            })
            .unwrap();
        let mut expected = vec!["Runtime".to_owned()];
        if let Some(notes) = notes.filter(|notes| !notes.is_empty()) {
            expected.push(format!("user_note: {}", notes.trim()));
        }
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({"id":"questions-a","result":{"answers":{"scope":{"answers":expected},"detail":{"answers":["Keep 한글 intact"]}}}})
        );
        let count = sent.0.borrow().len();
        assert!(
            runtime
                .execute_command(AgentCommand::RespondToActivity {
                    request: second,
                    response: ActivityResponse::UserInput(UserInput::from("again"))
                })
                .is_err()
        );
        assert_eq!(sent.0.borrow().len(), count);
        runtime.poll_event().unwrap();
        let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(receipt),
            ..
        }) = runtime.poll_event().unwrap()
        else {
            panic!("final answer receipt missing")
        };
        assert!(receipt.contains("Answer: Keep 한글 intact"));
        assert!(receipt.contains("All question responses sent."));
        assert!(!receipt.contains("remaining questions"));
        runtime.poll_event().unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: second.activity(),
                outcome: ActivityOutcome::Completed
            })
        );
    }
}

// 중복 질문 ID와 비밀 입력은 화면에 노출하거나 잘못된 답을 합성하지 않고 원래 wire ID로 거절한다.
#[test]
fn rejects_ambiguous_or_secret_questions_before_presentation() {
    for questions in [
        json!([{"id":"same","header":"A","question":"First?"},{"id":"same","header":"B","question":"Second?"}]),
        json!([{"id":"secret","header":"Secret","question":"Password?","isSecret":true}]),
        json!([{"id":"other","header":"Other","question":"Choose?","isOther":null}]),
        json!([{"id":"other","header":"Other","question":"Choose?","isOther":"true"}]),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":99,"method":"item/tool/requestUserInput","params":{"threadId":"thread-a","turnId":"turn-a","questions":questions}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("ask"),
            })
            .unwrap();
        assert!(backend.poll_event().is_err());
        let messages = sent.0.borrow();
        let rejected = messages.last().unwrap();
        assert_eq!(rejected["id"], 99);
        assert_eq!(rejected["error"]["code"], -32602);
        assert!(!rejected.to_string().contains("Password?"));
    }
}

// 중단·실패·일반 종료·원격 질문 닫힘은 실제 기록된 답과 미응답 질문을 한 번만 요약한다.
// 부분 답변 묶음을 전송하지 않고 요청을 닫으며 늦은 응답/종료가 이를 다시 열지 못한다.
#[test]
fn closed_interview_preserves_partial_summary_and_rejects_stale_responses() {
    use yo_core::{
        ActivityNotice, ActivityUpdate, AgentRejection, NoticeLevel, RuntimeError, TurnOutcome,
    };
    for closed in ["interrupted", "failed", "completed", "resolved"] {
        for recorded in [false, true] {
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            let closure = if closed == "resolved" {
                json!({"method":"serverRequest/resolved","params":{"threadId":"thread-a","requestId":"questions-a"}})
            } else {
                json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":closed,"error":{"message":"interview ended"}}}})
            };
            let (backend, sent) = backend([
                thread_start_response(2, "thread-a"),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"id":"questions-a","method":"item/tool/requestUserInput","params":{
                    "threadId":"thread-a","turnId":"turn-a","questions":[
                        {"id":"one","header":"One","question":"First?"},
                        {"id":"two","header":"Two","question":"Second?"}
                    ]
                }}),
                closure,
                json!({"method":"serverRequest/resolved","params":{"threadId":"thread-a","requestId":"questions-a"}}),
            ]);
            let mut runtime = AgentRuntime::new(backend);
            runtime
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: active_turn,
                        input: UserInput::from("ask"),
                    },
                    submission(1),
                )
                .unwrap();
            runtime.poll_event().unwrap();
            runtime.poll_event().unwrap();
            if recorded {
                runtime
                    .execute_command(AgentCommand::RespondToActivity {
                        request: ActivityRequestRef::new(
                            activity(active_turn, 1),
                            RequestId::new(id(1)),
                        ),
                        response: ActivityResponse::UserInput(UserInput::from("partial")),
                    })
                    .unwrap();
                for _ in 0..6 {
                    runtime.poll_event().unwrap();
                }
            }
            let current = ActivityRequestRef::new(
                activity(active_turn, if recorded { 3 } else { 1 }),
                RequestId::new(id(if recorded { 2 } else { 1 })),
            );
            let RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: ended,
                outcome,
            }) = runtime.poll_event().unwrap()
            else {
                panic!("request must close first")
            };
            assert_eq!(ended, current.activity());
            if closed == "failed" {
                assert!(matches!(outcome, ActivityOutcome::Failed(_)));
            } else {
                assert_eq!(outcome, ActivityOutcome::Interrupted);
            }
            assert!(matches!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityStarted {
                    kind: ActivityKind::ModelWork,
                    ..
                })
            ));
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = runtime.poll_event().unwrap()
            else {
                panic!("summary missing")
            };
            let summary = ActivityNotice::from_snapshot(&text).unwrap();
            assert_eq!(summary.title, "Interview incomplete");
            assert_eq!(summary.level, NoticeLevel::Warning);
            assert!(summary.message.contains(if recorded {
                "1/2 answers recorded · 1 unanswered"
            } else {
                "0/2 answers recorded · 2 unanswered"
            }));
            assert!(summary.message.contains(if recorded {
                "1. Recorded: First?"
            } else {
                "1. Unanswered: First?"
            }));
            assert!(summary.message.contains("2. Unanswered: Second?"));
            assert!(summary.message.contains("Submission incomplete."));
            assert!(!summary.message.contains("responses sent"));
            assert!(matches!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityFinished {
                    outcome: ActivityOutcome::Completed,
                    ..
                })
            ));
            if closed == "completed" {
                assert!(matches!(
                    runtime.poll_event(),
                    Err(RuntimeError::EventRejected {
                        rejection: AgentRejection::RequestStillUnanswered { .. },
                        ..
                    })
                ));
            } else if closed != "resolved" {
                let RuntimePoll::Event(AgentEvent::TurnFinished { turn, outcome }) =
                    runtime.poll_event().unwrap()
                else {
                    panic!("turn end missing")
                };
                assert_eq!(turn, active_turn);
                match closed {
                    "failed" => assert!(matches!(outcome, TurnOutcome::Failed(_))),
                    "completed" => assert_eq!(outcome, TurnOutcome::Completed),
                    _ => assert_eq!(outcome, TurnOutcome::Interrupted),
                }
            }
            if closed != "completed" {
                assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Pending);
            }
            let before = sent.0.borrow().len();
            assert!(
                runtime
                    .execute_command(AgentCommand::RespondToActivity {
                        request: current,
                        response: ActivityResponse::UserInput(UserInput::from("late"))
                    })
                    .is_err()
            );
            assert_eq!(sent.0.borrow().len(), before);
            assert!(
                !sent
                    .0
                    .borrow()
                    .iter()
                    .any(|message| message.get("id") == Some(&json!("questions-a")))
            );
        }
    }
}

// isOther는 선택지가 있을 때만 마지막 항목을 더하고 선택 결과를 원래 요청의 답변 이름으로 보낸다.
// 빈 선택지의 자유 입력과 64개 초과 시 읽을 수 있는 일반 질문 fallback도 유지한다.
#[test]
fn other_choice_matches_codex_semantics_and_preserves_free_text() {
    use yo_core::ActivityUpdate;
    for (count, other) in [(2, false), (2, true), (0, true), (64, true)] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let options = (1..=count)
            .map(|index| json!({"label":format!("Option {index}"),"description":"Description"}))
            .collect::<Vec<_>>();
        let (backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"other-question","method":"item/tool/requestUserInput","params":{"threadId":"thread-a","turnId":"turn-a","questions":[{"id":"choice","header":"Choice","question":"Choose?","isOther":other,"options":options}]}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: UserInput::from("ask"),
                },
                submission(1),
            )
            .unwrap();
        let RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::UserInputRequest { request_id },
        }) = runtime.poll_event().unwrap()
        else {
            panic!("question start")
        };
        let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(prompt),
            ..
        }) = runtime.poll_event().unwrap()
        else {
            panic!("question prompt")
        };
        let expected_count = count + usize::from(other && count > 0);
        if expected_count <= 64 {
            let profile = ActivityQuestion::from_snapshot(&prompt).unwrap();
            assert_eq!(profile.choices.len(), expected_count);
            assert_eq!(
                profile
                    .choices
                    .last()
                    .is_some_and(|choice| choice.label == "None of the above"),
                other && count > 0
            );
        } else {
            assert!(ActivityQuestion::from_snapshot(&prompt).is_none());
            assert!(prompt.contains("65. None of the above"));
        }
        let answer = if count == 0 {
            "Custom answer".to_owned()
        } else {
            expected_count.to_string()
        };
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request: ActivityRequestRef::new(activity, request_id),
                response: ActivityResponse::UserInput(UserInput::from(answer)),
            })
            .unwrap();
        let expected = if count == 0 {
            "Custom answer"
        } else if other {
            "None of the above"
        } else {
            "Option 2"
        };
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({"id":"other-question","result":{"answers":{"choice":{"answers":[expected]}}}})
        );
    }
}

// 선택지 0·첫 초과값·최댓값은 질문의 다른 답이나 메모로 바꾸지 않고 wire 전송 전에 거부한다.
#[test]
fn rejects_question_choice_outside_original_options_before_wire_response() {
    for choice in [0, 3, u32::MAX] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"question","method":"item/tool/requestUserInput","params":{
                "threadId":"thread-a","turnId":"turn-a","questions":[{"id":"area","header":"Area","question":"Choose","options":[{"label":"UI","description":"Layout"},{"label":"Runtime","description":"Events"}]}]
            }}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: UserInput::from("ask"),
                },
                submission(1),
            )
            .unwrap();
        runtime.poll_event().unwrap();
        runtime.poll_event().unwrap();
        let sent_before = sent.0.borrow().len();
        assert!(
            runtime
                .execute_command(AgentCommand::RespondToActivity {
                    request: ActivityRequestRef::new(
                        activity(active_turn, 1),
                        RequestId::new(id(1))
                    ),
                    response: ActivityResponse::QuestionAnswer {
                        choice,
                        notes: UserInput::from("literal note")
                    },
                })
                .is_err()
        );
        assert_eq!(sent.0.borrow().len(), sent_before);
    }
}

// 완료 표시의 정확한 출력 한도와 첫 초과 바이트를 확인한다. 초과 원문은 잘린 답처럼
// 보이지 않게 생략을 명시하며 선택 이름에 user_note 접두사가 있어도 메모로 재해석하지 않는다.
#[test]
fn answer_receipt_bounds_and_literal_fields_do_not_change_response_authority() {
    use serde_json::Map;
    use yo_core::ToolOutput;

    use super::super::{InputQuestion, InputQuestions};
    let questions = InputQuestions {
        questions: vec![InputQuestion {
            id: "q".into(),
            prompt: "Which?".into(),
            question: "Which?".into(),
            options: vec![],
            choices: vec![],
        }],
        current: 0,
        answers: Map::new(),
        drafts: Default::default(),
    };
    let literal = questions.receipt(
        "user_note: a literal option",
        Some("![image](data:image/png;base64,abc)\n```rust\ncode"),
    );
    assert!(literal.contains("Answer: user_note: a literal option\nNote: ![image]"));
    for note in [false, true] {
        let overhead = questions.receipt("", note.then_some("")).len();
        let mut text = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
        let exact = if note {
            questions.receipt("", Some(&text))
        } else {
            questions.receipt(&text, None)
        };
        assert_eq!(exact.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
        assert!(exact.contains(&text));
        text.push('x');
        let excess = if note {
            questions.receipt("", Some(&text))
        } else {
            questions.receipt(&text, None)
        };
        assert!(excess.contains("Answer display omitted"));
        assert!(excess.contains("All question responses sent."));
        assert!(excess.len() < 1024);
        assert!(questions.answers.is_empty());
    }
}

// wire 쓰기 실패는 전송 완료 표시를 만들거나 요청을 응답 완료로 바꾸지 않는다.
// 원래 실패와 이전 답변 수를 보존하되 답·메모나 확인되지 않은 최종 제출을 기록하지 않는다.
#[test]
fn failed_answer_transport_does_not_publish_a_sent_receipt() {
    use std::time::Duration;

    use serde_json::{Map, Value};
    use yo_backend::transport::JsonMessagePeer;
    use yo_core::{BackendFailure, BackendStopHandle};

    use super::super::{Backend, InputQuestion, InputQuestions, RequestBinding, RequestKind};
    use crate::{client::AppServerClient, transport::PeerPoll};
    struct RejectResponse;
    impl JsonMessagePeer for RejectResponse {
        fn stop_handle(&self) -> BackendStopHandle {
            BackendStopHandle::no_op()
        }
        fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
            assert_eq!(message["id"], json!("question"));
            assert_eq!(
                message["result"]["answers"]["q"]["answers"],
                json!(["Runtime", "user_note: exact note"])
            );
            Err(BackendFailure::new(
                BackendFailureKind::ProcessExit,
                "fixture write failed",
            ))
        }
        fn receive(&mut self, _: Duration) -> Result<PeerPoll, BackendFailure> {
            Ok(PeerPoll::Pending)
        }
        fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
            Ok(PeerPoll::Pending)
        }
        fn shutdown(&mut self) -> Result<(), BackendFailure> {
            Ok(())
        }
    }
    for recorded in [0, 2] {
        let mut backend = Backend::new_uninitialized(
            AppServerClient::new(RejectResponse, Duration::from_secs(1)),
            "/workspace".into(),
            false,
            None,
        );
        let request =
            ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
        backend.requests.insert(
            request,
            RequestBinding {
                file_approval: None,
                wire_id: json!("question"),
                request_activity: request.activity(),
                responded: false,
                kind: RequestKind::Input(InputQuestions {
                    drafts: Default::default(),
                    current: recorded,
                    answers: (0..recorded)
                        .map(|index| {
                            (
                                format!("earlier-{index}"),
                                json!({"answers":["private-earlier-value"]}),
                            )
                        })
                        .collect::<Map<_, _>>(),
                    questions: (0..=recorded)
                        .map(|index| InputQuestion {
                            id: if index == recorded {
                                "q".into()
                            } else {
                                format!("earlier-{index}")
                            },
                            prompt: "Choose".into(),
                            question: "Choose".into(),
                            options: vec!["Runtime".into()],
                            choices: vec![],
                        })
                        .collect(),
                }),
            },
        );
        let failure = backend
            .respond_to_activity(
                request,
                ActivityResponse::QuestionAnswer {
                    choice: 1,
                    notes: UserInput::from("exact note"),
                },
            )
            .unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
        assert_eq!(
            failure.message(),
            format!(
                "fixture write failed\nInterview incomplete: {recorded}/{} earlier answers recorded. Final submission was not confirmed.",
                recorded + 1
            )
        );
        assert!(!failure.message().contains("exact note"));
        assert!(!failure.message().contains("private-earlier-value"));
        assert!(backend.pending_events.is_empty());
        assert!(!backend.requests[&request].responded);
    }
}

// 요약의 마지막 허용 바이트는 전체 질문을 유지하고 첫 초과는 질문 목록 생략을 명시한다.
// 기록 수와 미응답 수는 생략 뒤에도 유지하며 JSON/Markdown처럼 생긴 질문도 원문이다.
#[test]
fn incomplete_interview_summary_preserves_counts_at_exact_output_limit() {
    use serde_json::Map;
    use yo_core::{ActivityNotice, ToolOutput};

    use super::super::{InputQuestion, InputQuestions};
    let mut questions = InputQuestions {
        questions: vec![InputQuestion {
            id: "q".into(),
            prompt: "Q".into(),
            question: "".into(),
            options: vec![],
            choices: vec![],
        }],
        current: 0,
        answers: Map::new(),
        drafts: Default::default(),
    };
    let base = questions.incomplete_notice("Turn interrupted.");
    questions.questions[0].question = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - base.len());
    let exact = questions.incomplete_notice("Turn interrupted.");
    assert_eq!(exact.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert!(
        ActivityNotice::from_snapshot(&exact)
            .unwrap()
            .message
            .contains(&questions.questions[0].question)
    );
    questions.questions[0].question.push('x');
    let excess =
        ActivityNotice::from_snapshot(&questions.incomplete_notice("Turn interrupted.")).unwrap();
    assert!(
        excess
            .message
            .contains("0/1 answers recorded · 1 unanswered")
    );
    assert!(excess.message.contains("Question list omitted"));
    assert!(excess.message.len() < 1024);
    questions.questions[0].question =
        "[literal](https://example.com)\n{\"schema\":\"untrusted\"}".into();
    let literal =
        ActivityNotice::from_snapshot(&questions.incomplete_notice("Turn interrupted.")).unwrap();
    assert!(literal.message.contains(&questions.questions[0].question));
    assert!(questions.answers.is_empty());
}

// 마지막 답을 전송한 뒤 Turn이 종료되어도 미응답 요약을 만들지 않는다. 늦은 resolved는 중복 종료가
// 아니다.
#[test]
fn completed_interview_turn_closes_sent_request_without_incomplete_summary() {
    use yo_core::{ActivityUpdate, TurnOutcome};
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"q","method":"item/tool/requestUserInput","params":{"threadId":"thread-a","turnId":"turn-a","questions":[{"id":"q","header":"Q","question":"Which?"}]}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        json!({"method":"serverRequest/resolved","params":{"threadId":"thread-a","requestId":"q"}}),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("ask"),
            },
            submission(1),
        )
        .unwrap();
    runtime.poll_event().unwrap();
    runtime.poll_event().unwrap();
    let request = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::UserInput(UserInput::from("answer")),
        })
        .unwrap();
    runtime.poll_event().unwrap();
    let RuntimePoll::Event(AgentEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(receipt),
        ..
    }) = runtime.poll_event().unwrap()
    else {
        panic!("receipt missing")
    };
    assert!(receipt.contains("All question responses sent."));
    runtime.poll_event().unwrap();
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed
        })
    );
    assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Pending);
}

// 승인 요청의 실행 위치·권한·네트워크·정책 제안과 미지 필드를 빠뜨리지 않고 표시한다.
// 제안은 표시 데이터이며 승인 응답은 원래 요청 ID의 accept/decline 결정 그대로다.
#[test]
fn approval_context_retains_reported_scope_without_applying_policy_proposals() {
    use yo_core::{BackendEvent, BackendPoll};
    for file_change in [false, true] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let params = json!({
            "threadId":"thread-a","turnId":"turn-a","itemId":"item-a",
            "command":"cargo test","cwd":"/workspace/nested","environmentId":"remote-build",
            "grantRoot":"/workspace/generated","reason":"Verify the generated output",
            "networkApprovalContext":{"host":"example.com","protocol":"https"},
            "additionalPermissions":{"network":{"enabled":true},"fileSystem":{"write":["/workspace/generated"]}},
            "proposedExecpolicyAmendment":["cargo","test"],
            "proposedNetworkPolicyAmendments":[{"host":"example.com","action":"allow"}],
            "commandActions":[{"type":"unknown","command":"cargo test"}],
            "availableDecisions":["accept","acceptForSession","decline"],
            "future\npermission":{"scope":"literal value","enabled":false}
        });
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"approval-context","method":if file_change {"item/fileChange/requestApproval"} else {"item/commandExecution/requestApproval"},"params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("review"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity: approval,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) = backend.poll_event().unwrap()
        else {
            panic!("approval request missing")
        };
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(text),
            ..
        }) = backend.poll_event().unwrap()
        else {
            panic!("approval context missing")
        };
        let text = ActivityApproval::from_snapshot(&text).unwrap().plain_text;
        for expected in [
            "Command: cargo test",
            "Working directory: /workspace/nested",
            "Environment: remote-build",
            "Network target:",
            "Additional permissions:",
            "Proposed command policy:",
            "Proposed network policies:",
            "Reported command actions:",
            "Offered decisions:",
            "example.com",
            "Additional field \"future\\npermission\": {\"enabled\":false,\"scope\":\"literal value\"}",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        if file_change {
            assert!(text.contains("Requested write root (session scope): /workspace/generated"));
        }
        assert!(!text.contains("thread-a"));
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request: ActivityRequestRef::new(approval, request_id),
                response: ActivityResponse::Approval(ApprovalDecision::Declined),
            })
            .unwrap();
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({"id":"approval-context","result":{"decision":"decline"}})
        );
        backend.poll_event().unwrap();
        assert!(
            matches!(backend.poll_event().unwrap(), BackendPoll::Event(BackendEvent::ActivityUpdated { update: ActivityUpdate::TextSnapshot(text), .. }) if text == "Decision: declined")
        );
    }
}

// 승인 문맥이 마지막 허용 바이트를 넘으면 부분 문맥이나 응답 가능한 binding을 만들지 않는다.
// JSON 직렬화가 커지는 미지 필드도 같은 제한을 거쳐 원래 wire ID로 오류를 반환한다.
#[test]
fn oversized_approval_context_is_rejected_before_a_request_can_be_approved() {
    use yo_core::{BackendEvent, BackendPoll, ToolOutput};
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let request_backend = |params: Value| {
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"large-approval","method":"item/commandExecution/requestApproval","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("review"),
            })
            .unwrap();
        (backend, sent)
    };
    let (mut probe, _) = request_backend(
        json!({"threadId":"thread-a","turnId":"turn-a","itemId":"a","command":"x"}),
    );
    probe.poll_event().unwrap();
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(snapshot),
        ..
    }) = probe.poll_event().unwrap()
    else {
        panic!("probe approval missing")
    };
    let encoded_overhead = snapshot.len() - 1;
    for mode in [0, 1, 2] {
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"a"});
        if mode < 2 {
            params["command"] =
                json!("x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - encoded_overhead + mode));
        } else {
            params["future"] = json!("\"".repeat(ToolOutput::MAX_SNAPSHOT_BYTES / 2));
        }
        let (mut backend, sent) = request_backend(params);
        if mode == 0 {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::ApprovalRequest { .. },
                    ..
                })
            ));
            let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
            else {
                panic!("exact approval context missing")
            };
            assert_eq!(text.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
            assert_eq!(backend.requests.len(), 1);
        } else {
            assert!(backend.poll_event().is_err());
            assert!(backend.requests.is_empty());
            assert!(backend.pending_events.is_empty());
            let messages = sent.0.borrow();
            let error = messages.last().unwrap();
            assert_eq!(error["id"], json!("large-approval"));
            assert_eq!(error["error"]["code"], json!(-32602));
        }
    }
}

// 제공되지 않은 결정을 요청별로 거부하고 wire 전송·응답 활동·binding 소비가 없음을 확인한다.
// 정책 객체 안의 accept 문자열이나 세션 승인을 일반 승인으로 바꾸지 않는다.
#[test]
fn approval_responses_are_limited_to_the_exact_offered_decisions() {
    use yo_core::{BackendEvent, BackendPoll};

    for offered in [
        json!(["decline"]),
        json!(["acceptForSession", "decline"]),
        json!([{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["accept"]}}, "decline"]),
        json!([{"futureDecision":"accept"}, "decline"]),
        json!([]),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"limited","method":"item/commandExecution/requestApproval", "params":{
                "threadId":"thread-a","turnId":"turn-a","itemId":"a",
                "availableDecisions":offered,
            }}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("review"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing request")
        };
        backend.poll_event().unwrap();
        let request = ActivityRequestRef::new(activity, request_id);
        let before = sent.0.borrow().len();
        let error = backend
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(ApprovalDecision::Approved),
            })
            .unwrap_err();
        assert!(error.to_string().contains("not offered"), "{error}");
        assert_eq!(sent.0.borrow().len(), before);
        assert!(!backend.requests[&request].responded);
        assert!(backend.pending_events.is_empty());
        let declined = backend.execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Declined),
        });
        if offered.as_array().unwrap().is_empty() {
            assert!(declined.is_err());
            assert_eq!(sent.0.borrow().len(), before);
            assert!(!backend.requests[&request].responded);
            assert!(backend.pending_events.is_empty());
        } else {
            declined.unwrap();
            assert_eq!(
                sent.0.borrow().last().unwrap(),
                &json!({
                    "id":"limited","result":{"decision":"decline"},
                })
            );
            assert!(backend.requests[&request].responded);
        }
    }
}

// 명시 목록이 없는 이전 서버는 일반 승인/거절을 유지하고 명시 목록은 각 요청에만 적용한다.
#[test]
fn approval_options_preserve_legacy_defaults_and_do_not_cross_request_bindings() {
    use yo_core::{BackendEvent, BackendPoll};

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"restricted","method":"item/commandExecution/requestApproval","params":{
            "threadId":"thread-a","turnId":"turn-a","itemId":"a","availableDecisions":["decline"],
        }}),
        json!({"id":"legacy","method":"item/fileChange/requestApproval","params":{
            "threadId":"thread-a","turnId":"turn-a","itemId":"b",
        }}),
        json!({"id":"nullable","method":"item/commandExecution/requestApproval","params":{
            "threadId":"thread-a","turnId":"turn-a","itemId":"c","availableDecisions":null,
        }}),
        json!({"id":"explicit","method":"item/commandExecution/requestApproval","params":{
            "threadId":"thread-a","turnId":"turn-a","itemId":"d","availableDecisions":["accept"],
        }}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("review"),
        })
        .unwrap();
    let mut requests = Vec::new();
    for _ in 0..4 {
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing request")
        };
        backend.poll_event().unwrap();
        requests.push(ActivityRequestRef::new(activity, request_id));
    }
    let before = sent.0.borrow().len();
    assert!(
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request: requests[3],
                response: ActivityResponse::Approval(ApprovalDecision::Declined),
            })
            .is_err()
    );
    assert_eq!(sent.0.borrow().len(), before);
    assert!(!backend.requests[&requests[3]].responded);
    assert!(backend.pending_events.is_empty());
    for (request, (wire_id, decision, expected)) in requests[1..].iter().zip([
        ("legacy", ApprovalDecision::Approved, "accept"),
        ("nullable", ApprovalDecision::Declined, "decline"),
        ("explicit", ApprovalDecision::Approved, "accept"),
    ]) {
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request: *request,
                response: ActivityResponse::Approval(decision),
            })
            .unwrap();
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({"id":wire_id,"result":{"decision":expected}})
        );
    }
    let before = sent.0.borrow().len();
    assert!(
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request: requests[0],
                response: ActivityResponse::Approval(ApprovalDecision::Approved),
            })
            .is_err()
    );
    assert_eq!(sent.0.borrow().len(), before);
    assert!(!backend.requests[&requests[0]].responded);
}

// 형식이 잘못된 결정 목록은 원래 요청 ID로 오류를 돌려주며 승인 UI용 요청을 게시하지 않는다.
#[test]
fn malformed_approval_options_fail_before_request_publication() {
    for offered in [json!("accept"), json!({"decision":"accept"}), json!(true)] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"malformed","method":"item/commandExecution/requestApproval","params":{
                "threadId":"thread-a","turnId":"turn-a","itemId":"a","availableDecisions":offered,
            }}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("review"),
            })
            .unwrap();
        assert!(backend.poll_event().is_err());
        assert!(backend.requests.is_empty());
        assert!(backend.wire_requests.is_empty());
        assert!(backend.pending_events.is_empty());
        let sent = sent.0.borrow();
        let response = sent.last().unwrap();
        assert_eq!(response["id"], "malformed");
        assert_eq!(response["error"]["code"], -32602);
    }
}

// 세션 승인·명령 규칙·네트워크 규칙은 표시한 원래 선택지 번호에서 정확한 wire 값으로 연결한다.
// 범위 밖 또는 미지원 결정은 전송 없이 거부하고 성공한 선택만 범위 설명을 기록한다.
#[test]
fn offered_approval_choices_preserve_exact_scopes_and_wire_payloads() {
    use yo_core::{BackendEvent, BackendPoll};
    let choices = json!([
        "accept", "acceptForSession", "decline", "cancel",
        {"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo", "test"]}},
        {"applyNetworkPolicyAmendment":{"network_policy_amendment":{"host":"example.com", "action":"allow"}}},
        {"applyNetworkPolicyAmendment":{"network_policy_amendment":{"host":"example.com", "action":"deny"}}},
        {"futureDecision":"accept"},
        {"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo"], "unexpected":"grant"}}
    ]);
    for ordinal in 1..=7 {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"scoped","method":"item/commandExecution/requestApproval","params":{
                "threadId":"thread-a","turnId":"turn-a","itemId":"a","command":"cargo test","availableDecisions":choices,
            }}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("review"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing request")
        };
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(snapshot),
            ..
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing choices")
        };
        let profile = ActivityApproval::from_snapshot(&snapshot).unwrap();
        assert_eq!(profile.decline_choice, Some(3));
        assert!(profile.choices[1].label.contains("session"));
        assert!(profile.choices[4].description.contains("Persistent"));
        assert!(profile.choices[5].description.contains("example.com"));
        assert!(!profile.choices[7].enabled);
        assert!(!profile.choices[8].enabled);
        let request = ActivityRequestRef::new(activity, request_id);
        let before = sent.0.borrow().len();
        for invalid in [0, 8, 9, 10, u32::MAX] {
            assert!(
                backend
                    .execute_command(AgentCommand::RespondToActivity {
                        request,
                        response: ActivityResponse::Approval(ApprovalDecision::Offered(invalid)),
                    })
                    .is_err()
            );
            assert_eq!(sent.0.borrow().len(), before);
            assert!(!backend.requests[&request].responded);
            assert!(backend.pending_events.is_empty());
        }
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(ApprovalDecision::Offered(ordinal)),
            })
            .unwrap();
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({"id":"scoped","result":{"decision":choices[ordinal as usize - 1]}})
        );
        backend.poll_event().unwrap();
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(receipt),
            ..
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing receipt")
        };
        assert!(receipt.contains(&profile.choices[ordinal as usize - 1].label));
        assert!(receipt.contains(&profile.choices[ordinal as usize - 1].description));
    }
}

// 파일 변경 요청에 명령 정책 객체가 섞여 있어도 이를 제출하지 않고 같은 요청의 정상 거절은
// 유지한다.
#[test]
fn file_approval_disables_command_policy_choices() {
    use yo_core::{BackendEvent, BackendPoll};
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"file","method":"item/fileChange/requestApproval","params":{
            "threadId":"thread-a","turnId":"turn-a","itemId":"a","availableDecisions":[
                {"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo"]}},"decline"
            ],
        }}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("review"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ApprovalRequest { request_id },
    }) = backend.poll_event().unwrap()
    else {
        panic!("missing request")
    };
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(snapshot),
        ..
    }) = backend.poll_event().unwrap()
    else {
        panic!("missing choices")
    };
    assert!(!ActivityApproval::from_snapshot(&snapshot).unwrap().choices[0].enabled);
    let request = ActivityRequestRef::new(activity, request_id);
    let before = sent.0.borrow().len();
    assert!(
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(ApprovalDecision::Offered(1))
            })
            .is_err()
    );
    assert_eq!(sent.0.borrow().len(), before);
    assert!(!backend.requests[&request].responded);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Offered(2)),
        })
        .unwrap();
    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({"id":"file","result":{"decision":"decline"}})
    );
}

// 목록 생략/null에서는 Codex의 요청별 기본 규칙을 적용하고 명시 목록(빈 목록 포함)이 항상 우선한다.
// 각 선택은 생성된 프로필의 원래 번호로 정확한 wire 값에 연결되며 정책은 제안된 경우에만 나타난다.
#[test]
fn legacy_approval_defaults_match_request_scope_and_preserve_wire_decisions() {
    use yo_core::{BackendEvent, BackendPoll};
    let command_rule =
        json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo","test"]}});
    let network_rule = json!({"applyNetworkPolicyAmendment":{"network_policy_amendment":{"host":"first.example","action":"allow"}}});
    let cases = [
        (false, json!({}), json!(["accept", "cancel"])),
        (
            false,
            json!({"availableDecisions":null}),
            json!(["accept", "cancel"]),
        ),
        (
            false,
            json!({"proposedExecpolicyAmendment":["cargo","test"]}),
            json!(["accept", command_rule, "cancel"]),
        ),
        (
            false,
            json!({"additionalPermissions":{},"proposedExecpolicyAmendment":["cargo","test"]}),
            json!(["accept", "cancel"]),
        ),
        (
            false,
            json!({"networkApprovalContext":{"host":"first.example","protocol":"https"},"additionalPermissions":{},"proposedExecpolicyAmendment":["cargo","test"],"proposedNetworkPolicyAmendments":[
                {"host":"blocked.example","action":"deny"},{"host":"first.example","action":"allow"},{"host":"second.example","action":"allow"}
            ]}),
            json!(["accept", "acceptForSession", network_rule, "cancel"]),
        ),
        (
            false,
            json!({"networkApprovalContext":{"host":"first.example","protocol":"https"},"proposedNetworkPolicyAmendments":[{"host":"first.example","action":"deny"}]}),
            json!(["accept", "acceptForSession", "cancel"]),
        ),
        (
            true,
            json!({"grantRoot":"/workspace/generated","proposedExecpolicyAmendment":["cargo","test"]}),
            json!(["accept", "acceptForSession", "cancel"]),
        ),
        (
            false,
            json!({"availableDecisions":["decline"],"networkApprovalContext":{"host":"first.example","protocol":"https"},"proposedExecpolicyAmendment":["cargo","test"]}),
            json!(["decline"]),
        ),
        (
            false,
            json!({"availableDecisions":[],"networkApprovalContext":true}),
            json!([]),
        ),
    ];
    for (file, fields, expected) in cases {
        let choices = expected.as_array().unwrap();
        for selected in 0..choices.len().max(1) {
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"a","command":"cargo test"});
            params
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let (mut backend, sent) = backend([
                thread_start_response(2, "thread-a"),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"id":"legacy-scoped","method":if file {"item/fileChange/requestApproval"} else {"item/commandExecution/requestApproval"},"params":params}),
            ]);
            backend
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: active_turn,
                    input: UserInput::from("review"),
                })
                .unwrap();
            let BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            }) = backend.poll_event().unwrap()
            else {
                panic!("missing request")
            };
            let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(snapshot),
                ..
            }) = backend.poll_event().unwrap()
            else {
                panic!("missing profile")
            };
            let profile = ActivityApproval::from_snapshot(&snapshot).unwrap();
            assert_eq!(profile.choices.len(), choices.len(), "{fields}");
            assert_eq!(
                profile.decline_choice,
                choices
                    .iter()
                    .position(|choice| matches!(choice.as_str(), Some("decline" | "cancel")))
                    .map(|index| index as u32 + 1)
            );
            let request = ActivityRequestRef::new(activity, request_id);
            let before = sent.0.borrow().len();
            for invalid in [0, choices.len() as u32 + 1, u32::MAX] {
                assert!(
                    backend
                        .execute_command(AgentCommand::RespondToActivity {
                            request,
                            response: ActivityResponse::Approval(ApprovalDecision::Offered(
                                invalid
                            ))
                        })
                        .is_err()
                );
                assert_eq!(sent.0.borrow().len(), before);
                assert!(backend.pending_events.is_empty());
                assert!(!backend.requests[&request].responded);
            }
            if choices.is_empty() {
                continue;
            }
            backend
                .execute_command(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(ApprovalDecision::Offered(
                        selected as u32 + 1,
                    )),
                })
                .unwrap();
            assert_eq!(
                sent.0.borrow().last().unwrap(),
                &json!({"id":"legacy-scoped","result":{"decision":choices[selected]}})
            );
            backend.poll_event().unwrap();
            let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(receipt),
                ..
            }) = backend.poll_event().unwrap()
            else {
                panic!("missing decision receipt")
            };
            assert!(receipt.contains(&profile.choices[selected].label));
            assert!(receipt.contains(&profile.choices[selected].description));
        }
    }
}

// 기본 선택지에 사용되는 잘못된 권한·정책 정보는 binding이나 부분 승인 화면을 게시하기 전에
// 거부한다.
#[test]
fn malformed_legacy_policy_hints_cannot_create_an_approvable_binding() {
    for fields in [
        json!({"proposedExecpolicyAmendment":"cargo test"}),
        json!({"proposedExecpolicyAmendment":["cargo",42]}),
        json!({"additionalPermissions":true}),
        json!({"networkApprovalContext":true}),
        json!({"networkApprovalContext":{"host":"example.com","protocol":"unknown"}}),
        json!({"networkApprovalContext":{"host":"example.com","protocol":"https"},"proposedNetworkPolicyAmendments":true}),
        json!({"networkApprovalContext":{"host":"example.com","protocol":"https"},"proposedNetworkPolicyAmendments":[{"host":"example.com","action":"unknown"}]}),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"a"});
        params
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let (mut backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"invalid-legacy","method":"item/commandExecution/requestApproval","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("review"),
            })
            .unwrap();
        assert!(backend.poll_event().is_err(), "{fields}");
        assert!(backend.requests.is_empty());
        assert!(backend.wire_requests.is_empty());
        assert!(backend.pending_events.is_empty());
        let sent = sent.0.borrow();
        let response = sent.last().unwrap();
        assert_eq!(response["id"], "invalid-legacy");
        assert_eq!(response["error"]["code"], -32602);
    }
}

// 연결 EOF와 수신 실패에서도 부분 답변 요약을 먼저 전달하고 원래 실패를 보존한다.
// 전송 완료한 질문에는 미제출 경고를 만들지 않고 닫힌 연결을 다시 읽지 않는다.
#[test]
fn connection_loss_delivers_interview_summary_before_terminal_failure() {
    use std::time::Duration;

    use yo_backend::transport::JsonMessagePeer;
    use yo_core::{AccountId, ActivityNotice, BackendFailure, BackendStopHandle, RuntimeError};

    use super::{
        super::Backend,
        support::{FakePeer, initialize_response},
    };
    use crate::{client::AppServerClient, transport::PeerPoll};

    struct EndingPeer {
        peer: FakePeer,
        terminal: Option<Result<PeerPoll, BackendFailure>>,
    }
    impl JsonMessagePeer for EndingPeer {
        fn stop_handle(&self) -> BackendStopHandle {
            self.peer.stop_handle()
        }
        fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
            self.peer.send(message)
        }
        fn receive(&mut self, timeout: Duration) -> Result<PeerPoll, BackendFailure> {
            self.peer.receive(timeout)
        }
        fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
            match self.peer.try_receive()? {
                PeerPoll::Pending => self.terminal.take().expect("terminal peer polled again"),
                other => Ok(other),
            }
        }
        fn shutdown(&mut self) -> Result<(), BackendFailure> {
            self.peer.shutdown()
        }
    }
    for failed in [false, true] {
        for recorded in 0..=2 {
            let (peer, sent) = FakePeer::new([
                initialize_response(1, "0.146.0"),
                thread_start_response(2, "thread-a"),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"id":"questions-a","method":"item/tool/requestUserInput","params":{
                    "threadId":"thread-a","turnId":"turn-a","questions":[
                        {"id":"one","header":"One","question":"First?"},
                        {"id":"two","header":"Two","question":"Second?"}
                    ]
                }}),
            ]);
            let peer = EndingPeer {
                peer,
                terminal: Some(if failed {
                    Err(BackendFailure::new(
                        BackendFailureKind::ProcessExit,
                        "fixture read failed",
                    ))
                } else {
                    Ok(PeerPoll::Closed)
                }),
            };
            let mut client = AppServerClient::new(peer, Duration::from_secs(1));
            let initialize = client.initialize().unwrap();
            let mut backend = Backend::new_uninitialized(client, "/workspace".into(), false, None);
            backend.initialized = true;
            backend.backend_version = Some(initialize.user_agent);
            backend.account = Some(AccountId::new("account-test").unwrap());
            let mut runtime = AgentRuntime::new(backend);
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            runtime
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: active_turn,
                        input: UserInput::from("ask"),
                    },
                    submission(1),
                )
                .unwrap();
            runtime.poll_event().unwrap();
            runtime.poll_event().unwrap();
            for index in 0..recorded {
                runtime
                    .execute_command(AgentCommand::RespondToActivity {
                        request: ActivityRequestRef::new(
                            activity(active_turn, if index == 0 { 1 } else { 3 }),
                            RequestId::new(id(index + 1)),
                        ),
                        response: ActivityResponse::UserInput(UserInput::from("answer")),
                    })
                    .unwrap();
                for _ in 0..if index == 0 { 6 } else { 3 } {
                    runtime.poll_event().unwrap();
                }
            }
            let mut summaries = Vec::new();
            let mut ended = false;
            for _ in 0..8 {
                match runtime.poll_event() {
                    Ok(RuntimePoll::Event(AgentEvent::ActivityUpdated {
                        update: ActivityUpdate::TextSnapshot(text),
                        ..
                    })) => {
                        if let Some(notice) = ActivityNotice::from_snapshot(&text) {
                            summaries.push(notice);
                        }
                    },
                    Ok(RuntimePoll::Event(_)) => {},
                    Err(RuntimeError::Backend { failure, .. }) => {
                        assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
                        if failed {
                            assert_eq!(failure.message(), "fixture read failed");
                        }
                        ended = true;
                        break;
                    },
                    other => panic!("unexpected connection result: {other:?}"),
                }
            }
            assert!(ended);
            assert_eq!(summaries.len(), usize::from(recorded < 2));
            if recorded < 2 {
                assert_eq!(summaries[0].title, "Interview incomplete");
                assert!(summaries[0].message.contains(&format!(
                    "{recorded}/2 answers recorded · {} unanswered",
                    2 - recorded
                )));
                assert!(summaries[0].message.contains("Connection to Codex ended"));
            }
            assert_eq!(
                sent.0
                    .borrow()
                    .iter()
                    .filter(|value| value["id"] == "questions-a")
                    .count(),
                usize::from(recorded == 2)
            );
            // 종료 후 재관찰은 중복 요약이나 추가 transport read를 만들지 않는다.
            if failed {
                assert!(matches!(
                    runtime.poll_event(),
                    Err(RuntimeError::Backend { .. })
                ));
            } else {
                assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Closed);
            }
        }
    }
}

// 승인 연결은 같은 턴의 실제 파일 변경 항목 ID로만 정하고 가장 최근 변경을 추측하지 않는다.
#[test]
fn file_approval_links_only_the_matching_observed_change() {
    for (item_id, expected) in [
        ("first", Some(1)),
        ("second", Some(2)),
        ("command", None),
        ("missing", None),
    ] {
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                "id":"first","type":"fileChange","changes":[{"path":"first.rs","kind":"update","diff":"@@ -1 +1 @@\n-old\n+first"}]}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                "id":"second","type":"fileChange","changes":[{"path":"second.rs","kind":"update","diff":"@@ -1 +1 @@\n-old\n+second"}]}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                "id":"command","type":"commandExecution","command":"true"}}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                "id":"first","type":"fileChange","status":"completed","changes":[{"path":"first.rs","kind":"update","diff":"@@ -1 +1 @@\n-old\n+first"}]}}}),
            json!({"id":"approval","method":"item/fileChange/requestApproval","params":{
                "threadId":"thread-a","turnId":"turn-a","itemId":item_id}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession {
                session_id: session(1),
            })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: turn(session(1), 1),
                    input: UserInput::from("edit"),
                },
                submission(1),
            )
            .unwrap();
        let mut found = false;
        for _ in 0..12 {
            if let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = runtime.poll_event().unwrap()
                && let Some(profile) = ActivityApproval::from_snapshot(&text)
            {
                assert_eq!(profile.related_change, expected);
                found = true;
                break;
            }
        }
        assert!(found);
    }
}

// 요청 뒤에 도착한 변경은 시작 또는 완료에서 본문이 관측된 뒤에만 연결하고 응답 완료 요청은
// 갱신하지 않는다.
#[test]
fn late_file_changes_update_only_the_matching_unanswered_approval() {
    for completed_body in [false, true] {
        for answered in [false, true] {
            let changes =
                json!([{"path":"target.rs","kind":"update","diff":"@@ -1 +1 @@\n-old\n+new"}]);
            let (backend, sent) = backend([
                thread_start_response(2, "thread-a"),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"id":"approval","method":"item/fileChange/requestApproval","params":{
                    "threadId":"thread-a","turnId":"turn-a","itemId":"target"}}),
                json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                    "id":"other","type":"fileChange","changes":changes}}}),
                json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                    "id":"target","type":"fileChange","changes":if completed_body {json!([])} else {changes.clone()}}}}),
                json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                    "id":"target","type":"fileChange","status":"completed","changes":changes}}}),
            ]);
            let mut runtime = AgentRuntime::new(backend);
            runtime
                .execute_command(AgentCommand::CreateSession {
                    session_id: session(1),
                })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: turn(session(1), 1),
                        input: UserInput::from("edit"),
                    },
                    submission(1),
                )
                .unwrap();
            runtime.poll_event().unwrap();
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = runtime.poll_event().unwrap()
            else {
                panic!("initial approval missing")
            };
            let initial = ActivityApproval::from_snapshot(&text).unwrap();
            assert_eq!(initial.related_change, None);
            let request =
                ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
            if answered {
                runtime
                    .execute_command(AgentCommand::RespondToActivity {
                        request,
                        response: ActivityResponse::Approval(ApprovalDecision::Approved),
                    })
                    .unwrap();
            }
            let mut updated = 0;
            let mut saw_target = false;
            for _ in 0..20 {
                match runtime.poll_event().unwrap() {
                    RuntimePoll::Pending => break,
                    RuntimePoll::Event(AgentEvent::ActivityUpdated {
                        activity: observed,
                        update: ActivityUpdate::TextSnapshot(text),
                    }) => {
                        if observed == activity(turn(session(1), 1), if answered { 4 } else { 3 }) {
                            saw_target = true;
                        }
                        if let Some(profile) = ActivityApproval::from_snapshot(&text) {
                            assert!(saw_target, "link must follow the visible target snapshot");
                            assert_eq!(observed, request.activity());
                            assert_eq!(profile.related_change, Some(3));
                            assert_eq!(profile.choices, initial.choices);
                            assert_eq!(profile.decline_choice, initial.decline_choice);
                            assert_eq!(profile.plain_text, initial.plain_text);
                            updated += 1;
                        }
                    },
                    _ => {},
                }
            }
            assert!(saw_target);
            assert_eq!(updated, usize::from(!answered));
            assert_eq!(
                sent.0
                    .borrow()
                    .iter()
                    .filter(|value| value["id"] == "approval")
                    .count(),
                usize::from(answered)
            );
            if !answered {
                runtime
                    .execute_command(AgentCommand::RespondToActivity {
                        request,
                        response: ActivityResponse::Approval(ApprovalDecision::Offered(3)),
                    })
                    .unwrap();
                assert!(
                    sent.0
                        .borrow()
                        .iter()
                        .any(|value| value
                            == &json!({"id":"approval","result":{"decision":"cancel"}}))
                );
            }
        }
    }
}

// 늦은 연결의 최대 ID 인코딩까지 포함한 마지막 바이트는 허용하고 첫 초과는 요청 binding 전에
// 거절한다.
#[test]
fn file_approval_reserves_capacity_for_later_change_link() {
    use yo_core::ToolOutput;
    let start = |reason: String| {
        let (backend, sent) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"id":"approval","method":"item/fileChange/requestApproval","params":{
                "threadId":"thread-a","turnId":"turn-a","itemId":"target","reason":reason}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{
                "id":"target","type":"fileChange","changes":[{"path":"target.rs","kind":"add","diff":"new"}]}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession {
                session_id: session(1),
            })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: turn(session(1), 1),
                    input: UserInput::from("edit"),
                },
                submission(1),
            )
            .unwrap();
        (runtime, sent)
    };
    let (mut probe, _) = start("x".into());
    probe.poll_event().unwrap();
    let RuntimePoll::Event(AgentEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(text),
        ..
    }) = probe.poll_event().unwrap()
    else {
        panic!("profile missing")
    };
    let mut profile = ActivityApproval::from_snapshot(&text).unwrap();
    profile.related_change = Some(u64::MAX);
    let overhead = profile.to_snapshot().unwrap().len() - 1;
    for excess in [0, 1] {
        let (mut runtime, sent) =
            start("x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead + excess));
        if excess == 1 {
            assert!(runtime.poll_event().is_err());
            assert!(
                sent.0
                    .borrow()
                    .iter()
                    .any(|value| value["id"] == "approval" && value["error"]["code"] == -32602)
            );
            assert!(
                !sent
                    .0
                    .borrow()
                    .iter()
                    .any(|value| value["id"] == "approval" && value.get("result").is_some())
            );
            continue;
        }
        runtime.poll_event().unwrap();
        runtime.poll_event().unwrap();
        runtime.poll_event().unwrap();
        runtime.poll_event().unwrap();
        let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(text),
            ..
        }) = runtime.poll_event().unwrap()
        else {
            panic!("late bounded profile missing")
        };
        let mut profile = ActivityApproval::from_snapshot(&text).unwrap();
        assert_eq!(profile.related_change, Some(2));
        profile.related_change = Some(u64::MAX);
        assert_eq!(
            profile.to_snapshot().unwrap().len(),
            ToolOutput::MAX_SNAPSHOT_BYTES
        );
    }
}

// 이전 질문 이동은 현재 선택·메모를 초안으로 보관하고 새 요청 ID로 다시 표시한다.
// 중간 이동·잘못된 선택·오래된 응답은 전송하지 않으며 최종 응답에는 수정된 답만 포함한다.
#[test]
fn revisits_questions_with_drafts_and_sends_only_final_answers() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"questions-nav","method":"item/tool/requestUserInput","params":{
            "threadId":"thread-a","turnId":"turn-a","questions":[
                {"id":"one","header":"One","question":"First?","options":[{"label":"A","description":"a"},{"label":"B","description":"b"}]},
                {"id":"two","header":"Two","question":"Second?","options":[{"label":"C","description":"c"},{"label":"D","description":"d"}]}
            ]
        }}),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("ask"),
            },
            submission(1),
        )
        .unwrap();
    let next_question = |runtime: &mut AgentRuntime<_>| {
        let mut request = None;
        for _ in 0..12 {
            match runtime.poll_event().unwrap() {
                RuntimePoll::Event(AgentEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::UserInputRequest { request_id },
                }) => {
                    request = Some(ActivityRequestRef::new(activity, request_id));
                },
                RuntimePoll::Event(AgentEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(text),
                }) => {
                    if let Some(question) = ActivityQuestion::from_snapshot(&text) {
                        let request = request.expect("fresh request before profile");
                        assert_eq!(request.activity(), activity);
                        return (request, question);
                    }
                },
                _ => {},
            }
        }
        panic!("question was not presented")
    };
    let (first, profile) = next_question(&mut runtime);
    assert!(!profile.previous_question);
    assert!(
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request: first,
                response: ActivityResponse::PreviousQuestion {
                    choice: None,
                    draft: UserInput::from("unsent")
                }
            })
            .is_err()
    );
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request: first,
            response: ActivityResponse::QuestionAnswer {
                choice: 1,
                notes: UserInput::from("first note"),
            },
        })
        .unwrap();
    let (second, profile) = next_question(&mut runtime);
    assert!(profile.previous_question);
    assert!(
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request: second,
                response: ActivityResponse::PreviousQuestion {
                    choice: Some(3),
                    draft: UserInput::from("invalid")
                }
            })
            .is_err()
    );
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request: second,
            response: ActivityResponse::PreviousQuestion {
                choice: Some(2),
                draft: UserInput::from("둘째\n/exit"),
            },
        })
        .unwrap();
    let (revisited, profile) = next_question(&mut runtime);
    assert_ne!(revisited, first);
    assert!(!profile.previous_question);
    assert_eq!(profile.draft.as_deref(), Some("first note"));
    assert_eq!(profile.draft_choice, Some(1));
    assert!(
        runtime
            .execute_command(AgentCommand::RespondToActivity {
                request: second,
                response: ActivityResponse::UserInput(UserInput::from("stale"))
            })
            .is_err()
    );
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request: revisited,
            response: ActivityResponse::QuestionAnswer {
                choice: 2,
                notes: UserInput::from("edited"),
            },
        })
        .unwrap();
    let (final_request, profile) = next_question(&mut runtime);
    assert_ne!(final_request, second);
    assert_eq!(profile.draft.as_deref(), Some("둘째\n/exit"));
    assert_eq!(profile.draft_choice, Some(2));
    assert!(
        !sent
            .0
            .borrow()
            .iter()
            .any(|message| message["id"] == "questions-nav")
    );
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request: final_request,
            response: ActivityResponse::QuestionAnswer {
                choice: profile.draft_choice.unwrap(),
                notes: UserInput::from(profile.draft.unwrap()),
            },
        })
        .unwrap();
    let messages = sent.0.borrow();
    let replies: Vec<_> = messages
        .iter()
        .filter(|message| message["id"] == "questions-nav")
        .collect();
    assert_eq!(
        replies,
        vec![&json!({"id":"questions-nav","result":{"answers":{
            "one":{"answers":["B","user_note: edited"]},
            "two":{"answers":["D","user_note: 둘째\n/exit"]}
        }}})]
    );
}

// 복원할 이전 초안이 전체 프로필의 정확한 한도에 맞을 때만 이동을 노출하고 첫 초과에서
// 비활성화한다.
#[test]
fn previous_question_availability_respects_restored_profile_limit() {
    use yo_core::ToolOutput;

    use super::super::{InputQuestion, InputQuestions};
    let mut questions = InputQuestions {
        questions: [("one", "First?"), ("two", "Second?")]
            .into_iter()
            .map(|(id, prompt)| InputQuestion {
                id: id.into(),
                prompt: prompt.into(),
                question: prompt.into(),
                options: vec![],
                choices: vec![],
            })
            .collect(),
        current: 0,
        answers: Default::default(),
        drafts: Default::default(),
    };
    questions.current = 1;
    questions.drafts.insert("one".into(), (None, String::new()));
    let overhead = questions.question_profile(0).to_snapshot().unwrap().len();
    questions.drafts.get_mut("one").unwrap().1 =
        "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
    assert_eq!(
        questions.question_profile(0).to_snapshot().unwrap().len(),
        ToolOutput::MAX_SNAPSHOT_BYTES
    );
    assert!(
        ActivityQuestion::from_snapshot(&questions.prompt())
            .unwrap()
            .previous_question
    );
    questions.drafts.get_mut("one").unwrap().1.push('x');
    let profile = ActivityQuestion::from_snapshot(&questions.prompt()).unwrap();
    assert!(!profile.previous_question);
    assert!(profile.plain_text.contains("Second?"));
    assert!(profile.draft.is_none());
}
