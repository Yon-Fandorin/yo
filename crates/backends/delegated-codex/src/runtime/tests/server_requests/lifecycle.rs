use super::{super::support::*, *};

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
    assert!(matches!(
        Capture::from_snapshot(&receipt).unwrap(),
        Capture::AcceptedAnswers { .. }
    ));
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

// 연결 EOF와 수신 실패에서도 부분 답변 요약을 먼저 전달하고 원래 실패를 보존한다.
// 전송 완료한 질문에는 미제출 경고를 만들지 않고 닫힌 연결을 다시 읽지 않는다.
#[test]
fn connection_loss_delivers_interview_summary_before_terminal_failure() {
    use std::time::Duration;

    use yo_backend::transport::JsonMessagePeer;
    use yo_core::{AccountId, ActivityNotice, BackendFailure, BackendStopHandle, RuntimeError};

    use super::super::{
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
