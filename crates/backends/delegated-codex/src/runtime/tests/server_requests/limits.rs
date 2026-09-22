use std::collections::HashMap;

use super::{super::support::*, input::display_question, *};

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

// 중복 ID와 잘못된 필드는 원래 wire ID로 거절하고, 유효한 secret 질문은 별도 입력 경로로
// presentation을 시작한다.
#[test]
fn rejects_ambiguous_or_secret_questions_before_presentation() {
    for (questions, accepted) in [
        (
            json!([{"id":"same","header":"A","question":"First?"},{"id":"same","header":"B","question":"Second?"}]),
            false,
        ),
        (
            json!([{"id":"secret","header":"Secret","question":"Password?","isSecret":true,"options":[]}]),
            true,
        ),
        (
            json!([{"id":"other","header":"Other","question":"Choose?","isOther":null}]),
            false,
        ),
        (
            json!([{"id":"other","header":"Other","question":"Choose?","isOther":"true"}]),
            false,
        ),
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
        if accepted {
            assert!(backend.poll_event().is_ok());
        } else {
            assert!(backend.poll_event().is_err());
        }
        let messages = sent.0.borrow();
        let rejected = messages.last().unwrap();
        if !accepted {
            assert_eq!(rejected["id"], 99);
            assert_eq!(rejected["error"]["code"], -32602);
            assert!(!rejected.to_string().contains("Password?"));
        }
    }
}

// secret prompt의 typed presentation이 출력 한도를 넘으면 Capture가 만들어져도 Activity
// start 전에 fail-closed한다. ordinary text fallback이나 부분 요청 binding은 허용하지 않는다.
#[test]
fn oversized_secret_presentation_is_rejected_before_activity_start() {
    use yo_core::ToolOutput;

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let oversized_header = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"oversized-secret","method":"item/tool/requestUserInput","params":{
            "threadId":"thread-a","turnId":"turn-a","questions":[
                {"id":"secret","header":oversized_header,"question":"Token?","isSecret":true,"options":[]}
            ]
        }}),
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
    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    let rejected = sent.0.borrow().last().unwrap().clone();
    assert_eq!(rejected["id"], "oversized-secret");
    assert_eq!(rejected["error"]["code"], -32602);
    assert!(!rejected.to_string().contains(&oversized_header));
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

    use super::super::super::{InputQuestion, InputQuestions};
    let questions = InputQuestions {
        capture: None,
        captured_answers: Vec::new(),
        questions: vec![InputQuestion {
            id: "q".into(),
            prompt: "Which?".into(),
            question: "Which?".into(),
            options: vec![],
            choices: vec![],
            is_secret: false,
        }],
        current: 0,
        answers: Map::new(),
        drafts: Default::default(),
        secret_delivery_blocked: false,
        probe_only: false,
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

    use super::super::super::{
        Backend, InputQuestion, InputQuestions, RequestBinding, RequestKind,
    };
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
                    capture: None,
                    captured_answers: Vec::new(),
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
                    secret_delivery_blocked: false,
                    probe_only: false,
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
                            is_secret: false,
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

// secret batch의 마지막 public answer write도 전송 결과가 모호하면 backend 오류 원문을
// 공개하지 않고, 보유한 secret을 지운 뒤 동일 request 재시도를 막는다.
#[test]
fn mixed_secret_transport_failure_is_static_and_non_retryable() {
    use std::time::Duration;

    use serde_json::{Map, Value};
    use yo_backend::transport::JsonMessagePeer;
    use yo_core::{BackendFailure, BackendStopHandle};

    use super::super::super::{
        Backend, InputQuestion, InputQuestions, RequestBinding, RequestKind,
    };
    use crate::{client::AppServerClient, transport::PeerPoll};

    struct RejectResponse;
    impl JsonMessagePeer for RejectResponse {
        fn stop_handle(&self) -> BackendStopHandle {
            BackendStopHandle::no_op()
        }
        fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
            assert_eq!(
                message["result"]["answers"]["secret"]["answers"],
                json!(["wire-secret"])
            );
            Err(BackendFailure::new(
                BackendFailureKind::ProcessExit,
                "transport echoed wire-secret",
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

    let session_id = session(1);
    let request = ActivityRequestRef::new(activity(turn(session_id, 1), 1), RequestId::new(id(1)));
    let mut backend = Backend::new_uninitialized(
        AppServerClient::new(RejectResponse, Duration::from_secs(1)),
        "/workspace".into(),
        false,
        None,
    );
    backend.requests.insert(
        request,
        RequestBinding {
            file_approval: None,
            wire_id: json!("mixed-secret"),
            request_activity: request.activity(),
            responded: false,
            kind: RequestKind::Input(InputQuestions {
                capture: None,
                captured_answers: Vec::new(),
                questions: vec![
                    InputQuestion {
                        id: "secret".into(),
                        prompt: "Secret".into(),
                        question: "Token?".into(),
                        options: vec![],
                        choices: vec![],
                        is_secret: true,
                    },
                    InputQuestion {
                        id: "public".into(),
                        prompt: "Public".into(),
                        question: "Name?".into(),
                        options: vec![],
                        choices: vec![],
                        is_secret: false,
                    },
                ],
                current: 1,
                answers: Map::from_iter([("secret".into(), json!({"answers":["wire-secret"]}))]),
                drafts: Default::default(),
                secret_delivery_blocked: false,
                probe_only: false,
            }),
        },
    );
    let failure = backend
        .respond_to_activity(
            request,
            ActivityResponse::UserInput(UserInput::from("public-answer")),
        )
        .unwrap_err();
    assert_eq!(
        failure.message(),
        "secret input delivery failed with an unknown outcome"
    );
    assert!(!failure.message().contains("wire-secret"));
    let RequestKind::Input(questions) = &backend.requests[&request].kind else {
        panic!("mixed input binding")
    };
    assert!(questions.secret_delivery_blocked);
    assert!(questions.answers.get("secret").is_none());
    let retry = backend
        .respond_to_activity(
            request,
            ActivityResponse::UserInput(UserInput::from("public-answer")),
        )
        .unwrap_err();
    assert!(retry.message().contains("cannot be retried"));
    assert!(!retry.message().contains("wire-secret"));
}

// 한 번이라도 secret 응답 경로에 들어간 backend는 request binding이 제거된 뒤에도 warning 원문을
// observer에 전달하지 않는다. thread 대상은 redaction 전에 확인하되 redacted 값에는 남기지 않는다.
#[test]
fn secret_response_taints_later_warning_observer_after_request_removal() {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use yo_core::{BackendPoll, SecretInput};

    use super::super::super::{
        Backend, InputQuestion, InputQuestions, RequestBinding, RequestKind,
    };
    use crate::client::AppServerClient;

    let (peer, _) = FakePeer::new([json!({
        "method": "guardianWarning",
        "params": {
            "threadId": "thread-a",
            "message": "raw-warning-secret",
            "details": "raw-details-secret",
            "path": "/raw/secret/path",
            "range": {"start": 7}
        }
    })]);
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&warnings);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1)).with_warning_observer(
        Some(Arc::new(move |warning| {
            observed.lock().unwrap().push(warning.to_string());
        })),
    );
    client.bind_notice_thread("thread-a");
    let mut backend = Backend::new_uninitialized(client, "/workspace".into(), false, None);
    let request = ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
    backend.requests.insert(
        request,
        RequestBinding {
            file_approval: None,
            wire_id: json!("secret-warning"),
            request_activity: request.activity(),
            responded: false,
            kind: RequestKind::Input(InputQuestions {
                capture: None,
                captured_answers: Vec::new(),
                questions: vec![InputQuestion {
                    id: "secret".into(),
                    prompt: "Secret".into(),
                    question: "Token?".into(),
                    options: vec![],
                    choices: vec![],
                    is_secret: true,
                }],
                current: 0,
                answers: Default::default(),
                drafts: Default::default(),
                secret_delivery_blocked: false,
                probe_only: false,
            }),
        },
    );
    backend
        .respond_to_activity(
            request,
            ActivityResponse::SecretInput(SecretInput::new("actual-secret").unwrap()),
        )
        .unwrap();
    backend.requests.remove(&request);
    backend.pending_events.clear();

    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    let warnings = warnings.lock().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0],
        "Codex warning (details redacted): Warning details are redacted after secret input."
    );
    for raw in [
        "raw-warning-secret",
        "raw-details-secret",
        "/raw/secret/path",
        "thread-a",
    ] {
        assert!(!warnings[0].contains(raw), "redacted warning leaked {raw}");
    }
}

// Backend shutdown이 transport 성공·실패 어느 쪽이어도 client.shutdown 전에 보류한 secret을
// 제거한다. public 답변은 shutdown 정리 대상이 아니므로 그대로 둔다.
#[test]
fn shutdown_discards_staged_secret_values_before_transport_shutdown() {
    use std::time::Duration;

    use serde_json::{Map, Value};
    use yo_backend::transport::JsonMessagePeer;
    use yo_core::{BackendFailure, BackendStopHandle};

    use super::super::super::{
        Backend, InputQuestion, InputQuestions, RequestBinding, RequestKind,
    };
    use crate::{client::AppServerClient, transport::PeerPoll};

    struct ShutdownPeer {
        failure: Option<BackendFailure>,
    }
    impl JsonMessagePeer for ShutdownPeer {
        fn stop_handle(&self) -> BackendStopHandle {
            BackendStopHandle::no_op()
        }
        fn send(&mut self, _: &Value) -> Result<(), BackendFailure> {
            Ok(())
        }
        fn receive(&mut self, _: Duration) -> Result<PeerPoll, BackendFailure> {
            Ok(PeerPoll::Pending)
        }
        fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
            Ok(PeerPoll::Pending)
        }
        fn shutdown(&mut self) -> Result<(), BackendFailure> {
            self.failure.take().map_or(Ok(()), Err)
        }
    }

    for failure in [None, Some("fixture shutdown failed")] {
        let request =
            ActivityRequestRef::new(activity(turn(session(1), 1), 1), RequestId::new(id(1)));
        let mut backend = Backend::new_uninitialized(
            AppServerClient::new(
                ShutdownPeer {
                    failure: failure.map(|message| {
                        BackendFailure::new(BackendFailureKind::ProcessExit, message)
                    }),
                },
                Duration::from_secs(1),
            ),
            "/workspace".into(),
            false,
            None,
        );
        backend.requests.insert(
            request,
            RequestBinding {
                file_approval: None,
                wire_id: json!("shutdown-secret"),
                request_activity: request.activity(),
                responded: false,
                kind: RequestKind::Input(InputQuestions {
                    capture: None,
                    captured_answers: Vec::new(),
                    questions: vec![
                        InputQuestion {
                            id: "secret".into(),
                            prompt: "Secret".into(),
                            question: "Token?".into(),
                            options: vec![],
                            choices: vec![],
                            is_secret: true,
                        },
                        InputQuestion {
                            id: "public".into(),
                            prompt: "Public".into(),
                            question: "Name?".into(),
                            options: vec![],
                            choices: vec![],
                            is_secret: false,
                        },
                    ],
                    current: 0,
                    answers: Map::from_iter([
                        ("secret".into(), json!({"answers":["staged-secret"]})),
                        ("public".into(), json!({"answers":["public-answer"]})),
                    ]),
                    drafts: HashMap::from([(
                        "secret".into(),
                        (None, "staged-secret-draft".into()),
                    )]),
                    secret_delivery_blocked: false,
                    probe_only: false,
                }),
            },
        );

        let result = backend.shutdown();
        if failure.is_some() {
            assert_eq!(result.unwrap_err().message(), "fixture shutdown failed");
        } else {
            result.unwrap();
        }
        let RequestKind::Input(questions) = &backend.requests[&request].kind else {
            panic!("shutdown secret input binding")
        };
        assert!(questions.answers.get("secret").is_none());
        assert!(!questions.drafts.contains_key("secret"));
        assert_eq!(
            questions.answers.get("public"),
            Some(&json!({"answers":["public-answer"]}))
        );
    }
}

// 요약의 마지막 허용 바이트는 전체 질문을 유지하고 첫 초과는 질문 목록 생략을 명시한다.
// 기록 수와 미응답 수는 생략 뒤에도 유지하며 JSON/Markdown처럼 생긴 질문도 원문이다.
#[test]
fn incomplete_interview_summary_preserves_counts_at_exact_output_limit() {
    use serde_json::Map;
    use yo_core::{ActivityNotice, ToolOutput};

    use super::super::super::{InputQuestion, InputQuestions};
    let mut questions = InputQuestions {
        capture: None,
        captured_answers: Vec::new(),
        questions: vec![InputQuestion {
            id: "q".into(),
            prompt: "Q".into(),
            question: "".into(),
            options: vec![],
            choices: vec![],
            is_secret: false,
        }],
        current: 0,
        answers: Map::new(),
        drafts: Default::default(),
        secret_delivery_blocked: false,
        probe_only: false,
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

// 복원할 이전 초안이 전체 프로필의 정확한 한도에 맞을 때만 이동을 노출하고 첫 초과에서
// 비활성화한다.
#[test]
fn previous_question_availability_respects_restored_profile_limit() {
    use yo_core::ToolOutput;

    use super::super::super::{InputQuestion, InputQuestions};
    let mut questions = InputQuestions {
        capture: None,
        captured_answers: Vec::new(),
        questions: [("one", "First?"), ("two", "Second?")]
            .into_iter()
            .map(|(id, prompt)| InputQuestion {
                id: id.into(),
                prompt: prompt.into(),
                question: prompt.into(),
                options: vec![],
                choices: vec![],
                is_secret: false,
            })
            .collect(),
        current: 0,
        answers: Default::default(),
        drafts: Default::default(),
        secret_delivery_blocked: false,
        probe_only: false,
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
        display_question(&questions.prompt())
            .unwrap()
            .previous_question
    );
    questions.drafts.get_mut("one").unwrap().1.push('x');
    let profile = display_question(&questions.prompt()).unwrap();
    assert!(!profile.previous_question);
    assert!(profile.plain_text.contains("Second?"));
    assert!(profile.draft.is_none());
}

// live secret 보유량은 질문별 64KiB 한도와 별도로 배치 전체 256KiB에서 닫힌다.
// 첫 초과 입력은 잘라서 저장하지 않고 기존 네 개의 exact 값만 유지한다.
#[test]
fn retained_secret_batch_limit_rejects_without_mutation() {
    use serde_json::Map;
    use yo_core::{BackendFailureKind, SecretInput};

    use super::super::super::{InputQuestion, InputQuestions};
    let mut questions = InputQuestions {
        capture: None,
        captured_answers: vec![None; 5],
        questions: (0..5)
            .map(|index| InputQuestion {
                id: format!("secret-{index}"),
                prompt: "Secret".into(),
                question: "Value?".into(),
                options: vec![],
                choices: vec![],
                is_secret: true,
            })
            .collect(),
        current: 0,
        answers: Map::new(),
        drafts: Default::default(),
        secret_delivery_blocked: false,
        probe_only: false,
    };
    let exact = "x".repeat(SecretInput::MAX_BYTES);
    for index in 0..4 {
        questions
            .retain_secret(index, &SecretInput::new(exact.clone()).unwrap())
            .unwrap();
    }
    assert_eq!(questions.answers.len(), 4);
    let failure = questions
        .retain_secret(4, &SecretInput::new("z").unwrap())
        .unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::InputOverBudget);
    assert!(failure.message().contains("256 KiB"));
    assert!(!failure.message().contains('z'));
    assert_eq!(questions.answers.len(), 4);
    assert!(!questions.answers.contains_key("secret-4"));
}

// secret question에서 ordinary PreviousQuestion draft/choice로 값을 우회할 수 없고,
// 거절된 navigation은 binding의 secret/draft 상태를 바꾸지 않는다.
#[test]
fn secret_question_navigation_rejects_ordinary_draft_bytes() {
    use serde_json::Map;

    use super::super::super::{InputQuestion, InputQuestions, RequestBinding, RequestKind};

    let session_id = session(1);
    let request = ActivityRequestRef::new(activity(turn(session_id, 1), 1), RequestId::new(id(1)));
    let mut backend = backend([]).0;
    backend.requests.insert(
        request,
        RequestBinding {
            file_approval: None,
            wire_id: json!("navigation-secret"),
            request_activity: request.activity(),
            responded: false,
            kind: RequestKind::Input(InputQuestions {
                capture: None,
                captured_answers: vec![None, None],
                questions: vec![
                    InputQuestion {
                        id: "public".into(),
                        prompt: "Public".into(),
                        question: "Public?".into(),
                        options: vec![],
                        choices: vec![],
                        is_secret: false,
                    },
                    InputQuestion {
                        id: "secret".into(),
                        prompt: "Secret".into(),
                        question: "Secret?".into(),
                        options: vec![],
                        choices: vec![],
                        is_secret: true,
                    },
                ],
                current: 1,
                answers: Map::from_iter([(
                    "secret".into(),
                    json!({"answers":["retained-secret"]}),
                )]),
                drafts: Default::default(),
                secret_delivery_blocked: false,
                probe_only: false,
            }),
        },
    );
    let failure = backend
        .respond_to_activity(
            request,
            ActivityResponse::PreviousQuestion {
                choice: None,
                draft: UserInput::from("ordinary-draft-bytes"),
            },
        )
        .unwrap_err();
    assert!(failure.message().contains("ordinary draft"));
    let RequestKind::Input(questions) = &backend.requests[&request].kind else {
        panic!("secret navigation binding")
    };
    assert!(questions.drafts.is_empty());
    assert!(questions.answers.contains_key("secret"));
    assert!(
        backend
            .respond_to_activity(
                request,
                ActivityResponse::PreviousQuestion {
                    choice: Some(1),
                    draft: UserInput::new(""),
                },
            )
            .is_err()
    );
}
// wire 전송 성공 뒤 capture 상한 초과는 final seal을 위조하지 않고 회복 불가 원인을 명시한다.
#[test]
fn oversized_final_interview_capture_exposes_unavailability_after_wire_success() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"seal-limit","method":"item/tool/requestUserInput","params":{"threadId":"thread-a","turnId":"turn-a","questions":[{"id":"q","header":"Q","question":"Answer?","options":null}]}}),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::new("ask"),
            },
            submission(1),
        )
        .unwrap();
    runtime.poll_event().unwrap();
    runtime.poll_event().unwrap();
    let request = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));
    let answer = "x".repeat(CAPTURE_LIMIT + 1);
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::UserInput(UserInput::new(answer.clone())),
        })
        .unwrap();
    assert_eq!(
        sent.0.borrow().last().unwrap()["result"]["answers"]["q"]["answers"][0],
        answer
    );
    let mut diagnosed = false;
    for _ in 0..3 {
        if let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(text),
            ..
        }) = runtime.poll_event().unwrap()
        {
            assert!(text.starts_with(RECOVERY_UNAVAILABLE_RECEIPT_PREFIX));
            assert!(text.contains("capture exceeds"));
            diagnosed = true;
        }
    }
    assert!(diagnosed);
    let catalog = runtime.transcript_reader().interviews();
    assert!(catalog.interviews()[0].submitted.is_none());
    assert!(
        catalog
            .recovery_unavailable(request)
            .is_some_and(|reason| reason.len() < 4096)
    );
}
