use super::{super::support::*, *};

// 재개된 스레드에서도 서버 버전과 시작된 도구의 이름·인자가 요청과 정확히 맞아야 한다.
#[test]
fn secret_entry_probe_rejects_unreviewed_wire_and_mismatched_tool_call() {
    use crate::runtime::secret_probe;

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let arguments = json!({});
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{
            "threadId":"thread-a", "turnId":"turn-a",
            "item":{"id":"probe-call","type":"dynamicToolCall","tool":secret_probe::TOOL_NAME,
                "arguments":arguments,"status":"inProgress"}
        }}),
        json!({"method":"item/started","params":{
            "threadId":"thread-a", "turnId":"turn-a",
            "item":{"id":"other-call","type":"dynamicToolCall","tool":"another_tool",
                "arguments":arguments,"status":"inProgress"}
        }}),
        json!({"method":"item/started","params":{
            "threadId":"thread-a", "turnId":"turn-a",
            "item":{"id":"completed-call","type":"dynamicToolCall","tool":secret_probe::TOOL_NAME,
                "arguments":arguments,"status":"inProgress"}
        }}),
        json!({"method":"item/completed","params":{
            "threadId":"thread-a", "turnId":"turn-a",
            "item":{"id":"completed-call","type":"dynamicToolCall","tool":secret_probe::TOOL_NAME,
                "arguments":arguments,"status":"completed"}
        }}),
    ]);
    backend.secret_probe_enabled = true;
    backend.backend_version = Some("yo/0.155.1 (test)".into());
    backend.create_session(session_id).unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test hidden input"),
        })
        .unwrap();
    for _ in 0..8 {
        if backend.items.len() == 2 {
            break;
        }
        backend.poll_event().unwrap();
    }
    assert_eq!(backend.items.len(), 2);
    let mut request = json!({
        "callId":"probe-call", "turnId":"turn-a", "tool":secret_probe::TOOL_NAME,
        "arguments":arguments
    });
    backend.backend_version = Some("other_cli/0.155.1 (test)".into());
    assert!(secret_probe::parse_request(&mut backend, &request).is_err());
    backend.backend_version = Some("yo/0.156.0 (test)".into());
    assert!(secret_probe::parse_request(&mut backend, &request).is_err());
    backend.backend_version = Some("yo/0.155.1 (test)".into());

    request["callId"] = json!("other-call");
    assert!(secret_probe::parse_request(&mut backend, &request).is_err());
    request["callId"] = json!("probe-call");

    request["arguments"]["question"] = json!("Ask for a real credential");
    assert!(secret_probe::parse_request(&mut backend, &request).is_err());
    request["arguments"] = json!({});
    assert!(secret_probe::parse_request(&mut backend, &request).is_ok());
    assert!(secret_probe::parse_request(&mut backend, &request).is_err());

    let mut saw_completed_call_start = false;
    for _ in 0..12 {
        backend.poll_event().unwrap();
        saw_completed_call_start |= backend.items.contains_key("completed-call");
        if saw_completed_call_start && !backend.items.contains_key("completed-call") {
            break;
        }
    }
    assert!(saw_completed_call_start);
    assert!(!backend.items.contains_key("completed-call"));
    request["callId"] = json!("completed-call");
    assert!(secret_probe::parse_request(&mut backend, &request).is_err());
}

// 동적 진단 도구는 기존 숨김 입력 화면을 열지만, 입력값 대신 고정된 공개 상태만
// Codex에 돌려줍니다. 모의값은 질문 캡처나 도구 결과에도 나타나지 않아야 합니다.
#[test]
fn secret_entry_probe_omits_sample_from_dynamic_tool_result() {
    use yo_core::SecretInput;

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{
            "threadId":"thread-a","turnId":"turn-a",
            "item":{"id":"probe-call","type":"dynamicToolCall","tool":"yo_secret_entry_probe",
                "arguments":{},"status":"inProgress"}
        }}),
        json!({"id":"probe-request","method":"item/tool/call","params":{
            "threadId":"thread-a","turnId":"turn-a","callId":"probe-call",
            "tool":"yo_secret_entry_probe",
            "arguments":{}
        }}),
    ]);
    backend.secret_probe_enabled = true;
    backend.backend_version = Some("yo/0.155.1 (test)".into());
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("test hidden input"),
            },
            submission(1),
        )
        .unwrap();
    let mut request = None;
    let mut prompt = None;
    for _ in 0..6 {
        match runtime.poll_event().unwrap() {
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            }) => request = Some(ActivityRequestRef::new(activity, request_id)),
            RuntimePoll::Event(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(value),
                ..
            }) if display_question(&value).is_some() => prompt = Some(value),
            _ => {},
        }
        if request.is_some() && prompt.is_some() {
            break;
        }
    }
    let question = display_question(&prompt.expect("hidden question")).unwrap();
    assert!(question.is_secret);
    assert!(question.plain_text.contains("discard"));
    assert!(question.plain_text.contains("Never enter a real password"));
    assert!(question.plain_text.contains("fixed completion status"));
    let sample = "sample-canary-한글-123";
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request: request.expect("probe request"),
            response: ActivityResponse::SecretInput(SecretInput::new(sample).unwrap()),
        })
        .unwrap();
    let sent = sent.0.borrow();
    let result = sent.last().unwrap();
    assert_eq!(result["id"], "probe-request");
    assert_eq!(result["result"]["success"], true);
    assert_eq!(result["result"]["contentItems"][0]["type"], "inputText");
    assert!(!serde_json::to_string(&*sent).unwrap().contains(sample));
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
        let Capture::Batch { questions, .. } = Capture::from_snapshot(&prompt).unwrap() else {
            panic!("complete interview capture")
        };
        assert!(questions[0].prompt.starts_with("Scope\n\nWhich area?"));
        assert!(questions[0].prompt.contains("2. Runtime — Events"));
        assert_eq!(questions[0].question, "Which area?");
        assert_eq!(questions[0].options[1].label, "Runtime");
        let question = display_question(&prompt).unwrap();
        assert_eq!(question.choices[1].label, "Runtime");
        assert_eq!(question.choices[1].description, "Events");
        assert!(question.plain_text.contains(
            "This answer is recorded locally. All answers are sent after the final question."
        ));
        assert!(question.plain_text.contains("Question 1 of 2"));
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
            matches!(&continuation[5],RuntimePoll::Event(AgentEvent::ActivityUpdated{activity,update:ActivityUpdate::TextSnapshot(text)}) if *activity==second.activity() && display_question(text).unwrap().plain_text.contains("Question 2 of 2") && display_question(text).unwrap().plain_text.contains("Submitting this answer sends all 2 answers."))
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
        let Capture::AcceptedAnswers {
            answers,
            final_request,
            ..
        } = Capture::from_snapshot(&receipt).unwrap()
        else {
            panic!("final genuine answer capture")
        };
        assert_eq!(final_request, second);
        assert_eq!(answers[1].text, "Keep 한글 intact");
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

// secret 질문은 고정된 공개 상태와 별도 live response로만 표시·전송된다. 입력 원문은
// backend wire 결과 외의 영수증, 프롬프트 또는 진단 텍스트에 들어가지 않는다.
#[test]
fn secret_question_uses_redacted_presentation_and_exact_wire_value() {
    use yo_core::{ActivityUpdate, SecretInput};

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":"secret-question","method":"item/tool/requestUserInput","params":{
            "threadId":"thread-a","turnId":"turn-a","questions":[
                {"id":"token","header":"Token","question":"Enter token","isSecret":true,"options":[]}
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
    let RuntimePoll::Event(AgentEvent::ActivityStarted {
        activity,
        kind: ActivityKind::UserInputRequest { request_id },
    }) = runtime.poll_event().unwrap()
    else {
        panic!("secret question start")
    };
    let RuntimePoll::Event(AgentEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(prompt),
        ..
    }) = runtime.poll_event().unwrap()
    else {
        panic!("secret question prompt")
    };
    let profile = display_question(&prompt).expect("secret profile");
    assert!(profile.is_secret);
    assert!(!profile.allow_notes);
    assert!(profile.choices.is_empty());
    assert!(profile.draft.is_none());
    assert!(profile.draft_choice.is_none());
    let request = ActivityRequestRef::new(activity, request_id);
    let value = "line-one\n/exit @literal $literal";
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new(value).unwrap()),
        })
        .unwrap();
    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({"id":"secret-question","result":{"answers":{"token":{"answers":[value]}}}})
    );
    for _ in 0..6 {
        if let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(text),
            ..
        }) = runtime.poll_event().unwrap()
        {
            assert!(!text.contains(value));
            assert!(!text.contains("line-one"));
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
        let Capture::Batch { questions, .. } = Capture::from_snapshot(&prompt).unwrap() else {
            panic!("complete interview capture")
        };
        assert_eq!(questions[0].options.len(), expected_count);
        if expected_count <= 64 {
            let profile = display_question(&prompt).unwrap();
            assert_eq!(profile.choices.len(), expected_count);
            assert_eq!(
                profile
                    .choices
                    .last()
                    .is_some_and(|choice| choice.label == "None of the above"),
                other && count > 0
            );
        } else {
            assert!(display_question(&prompt).is_none());
            assert_eq!(questions[0].options[64].label, "None of the above");
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
    fn next_question<P: JsonMessagePeer>(
        runtime: &mut AgentRuntime<super::super::super::Backend<P>>,
    ) -> (ActivityRequestRef, yo_core::ActivityQuestion) {
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
                    if let Some(mut question) = display_question(&text) {
                        let request = request.expect("fresh request before profile");
                        assert_eq!(request.activity(), activity);
                        if let Some(super::super::super::RequestBinding {
                            kind: super::super::super::RequestKind::Input(questions),
                            ..
                        }) = runtime.backend().requests.get(&request)
                        {
                            question = questions.question_profile(questions.current);
                        }
                        return (request, question);
                    }
                },
                _ => {},
            }
        }
        panic!("question was not presented")
    }
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

// 실제 질문 Activity에서 전달된 캡처를 기존 UI 프로필과 비교한다. 일반 텍스트의 표지는 권한이
// 아니다.
pub(super) fn display_question(text: &str) -> Option<yo_core::ActivityQuestion> {
    use yo_core::interview::Capture;
    if let Some(profile) = yo_core::ActivityQuestion::from_snapshot(text) {
        return Some(profile);
    }
    let capture = Capture::from_snapshot(text).ok()?;
    match capture {
        Capture::Batch { questions, .. } => {
            let q = questions.first()?;
            (q.options.len() <= 64).then(|| q.presentation(0, questions.len()))
        },
        Capture::Question { question, .. } => {
            (question.options.len() <= 64).then(|| question.presentation(1, 2))
        },
        _ => None,
    }
}
