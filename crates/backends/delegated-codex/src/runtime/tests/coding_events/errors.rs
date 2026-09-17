use super::{super::support::*, *};

// Codex의 error 알림은 곧바로 transport 실패로 중복 보고하지 않고 뒤따르는 failed
// turn/completed의 정확한 실패 메시지로 합쳐지는지 확인한다.
#[test]
fn folds_an_error_notification_into_the_failed_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "error",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "error": { "message": "model stream failed" }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": {
                    "id": "turn-a",
                    "status": "failed",
                    "error": { "message": "less specific" }
                }
            }
        }),
    ];
    let (backend, _) = backend(messages);
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
            submission(4),
        )
        .unwrap();

    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Failed(yo_core::Failure::new("model stream failed")),
        })
    );
}

// 요약 delta도 기존 thread/turn/item 경계를 따르고 잘못된 문단 번호나 다른 item 종류를 거절한다.
#[test]
fn rejects_invalid_public_summary_targets_and_indices() {
    for (item_type, patch) in [
        ("reasoning", json!({"threadId":"other"})),
        ("reasoning", json!({"turnId":"other"})),
        ("reasoning", json!({"itemId":"other"})),
        ("reasoning", json!({"summaryIndex":-1})),
        ("reasoning", json!({"summaryIndex":"0"})),
        ("agentMessage", json!({})),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"reason","summaryIndex":0,"delta":"text"});
        params
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"reason","type":item_type}}}),
            json!({"method":"item/reasoning/summaryTextDelta","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        backend.poll_event().unwrap();
        assert!(backend.poll_event().is_err(), "{item_type}: {patch}");
    }
}

// 재시도 가능 오류는 즉시 별도 안내로 전달하고 이후 실제 완료·실패 사유를 오염시키지 않는다.
#[test]
fn retry_notice_does_not_replace_the_terminal_failure() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};

    for status in ["completed", "failed"] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"error","params":{"threadId":"thread-a","turnId":"turn-a","willRetry":true,"error":{"message":"temporary disconnect"}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"terminal cause"}}}}),
            json!({"method":"error","params":{"threadId":"thread-a","turnId":"turn-a","willRetry":true,"error":{"message":"late retry"}}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }) = backend.poll_event().unwrap()
        else {
            panic!("retry notice must start")
        };
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: updated,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("retry text must arrive before completion")
        };
        assert_eq!(activity, updated);
        let notice = ActivityNotice::from_snapshot(&text).unwrap();
        assert_eq!(notice.title, "Retry announced");
        assert_eq!(notice.level, NoticeLevel::Warning);
        assert!(notice.message.contains("temporary disconnect"));
        assert_eq!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed
            })
        );
        match (status, backend.poll_event().unwrap()) {
            ("completed", BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. })) => {},
            (
                "failed",
                BackendPoll::Event(BackendEvent::TurnFinished {
                    outcome: TurnOutcome::Failed(failure),
                    ..
                }),
            ) => assert_eq!(failure.message(), "terminal cause"),
            other => panic!("unexpected terminal event: {other:?}"),
        }
        assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    }
}

// status 없는 압축 항목은 알림 종류로 시작·완료를 구분하고 같은 Activity와 실제 Turn을 유지한다.
#[test]
fn context_compaction_reports_progress_and_completion_without_invented_metrics() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};

    let session_id = session(1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    let active_turn = turn(session_id, 1);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("continue"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("compaction must start a visible activity");
    };
    assert_eq!(activity.turn(), active_turn);
    for title in ["Compacting context", "Context compacted"] {
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: updated,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing compaction snapshot");
        };
        assert_eq!(activity, updated);
        let notice = ActivityNotice::from_snapshot(&text).unwrap();
        assert_eq!(notice.title, title);
        assert_eq!(notice.level, NoticeLevel::Info);
        assert!(!notice.message.contains("tokens"));
        assert!(!notice.message.contains("summary"));
    }
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed
        })
    );
    assert!(
        matches!(backend.poll_event().unwrap(), BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) if turn == active_turn)
    );
}

// 압축 도중 중단되면 완료 안내를 합성하지 않고 뒤늦은 item 완료도 다시 표시하지 않는다.
#[test]
fn context_compaction_interruption_does_not_claim_completion() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};

    let session_id = session(1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"interrupted"}}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("continue"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted { activity, .. }) =
        backend.poll_event().unwrap()
    else {
        panic!("missing compaction activity");
    };
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(_),
            ..
        })
    ));
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Interrupted
        })
    );
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::TurnFinished {
            outcome: TurnOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// 백그라운드 터미널 대기·입력은 명령 완료 뒤에도 별도 안내로 남고 stdout이나 명령 실행을 합성하지
// 않는다.
#[test]
fn terminal_interactions_preserve_literal_input_after_command_completion() {
    use yo_core::{ActivityDocument, ActivityUpdate, BackendEvent, BackendPoll};
    let session_id = session(1);
    let input = "```\n![literal](data:image/png;base64,AAAA)\n\u{3}";
    let interaction = |stdin| json!({"method":"item/commandExecution/terminalInteraction","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"command-a","processId":"process-7","stdin":stdin}});
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"command-a","type":"commandExecution","command":"cargo test","cwd":"/workspace"}}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"command-a","type":"commandExecution","command":"cargo test","aggregatedOutput":"actual stdout","status":"completed"}}}),
        interaction(""),
        interaction(input),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        interaction("late input"),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let mut documents = Vec::new();
    let mut tool_starts = 0;
    let mut output = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        match event {
            BackendEvent::ActivityStarted {
                kind: ActivityKind::ToolCall,
                ..
            } => tool_starts += 1,
            BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            } => {
                if let Some(document) = ActivityDocument::from_snapshot(&text) {
                    documents.push(document);
                } else {
                    output.push(text);
                }
            },
            _ => {},
        }
    }
    assert_eq!(tool_starts, 1);
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].title, "Waited for background terminal");
    assert!(!documents[0].markdown.contains("Input:"));
    assert_eq!(documents[1].title, "Terminal input sent");
    assert!(documents[1].markdown.starts_with("````text\n"));
    assert!(documents[1].markdown.contains(input));
    assert!(documents[1].markdown.contains("Process: process-7"));
    assert!(documents[1].markdown.contains("Command: cargo test"));
    assert!(
        ToolOutput::from_snapshot(output.last().unwrap())
            .unwrap()
            .plain_text
            .ends_with("actual stdout")
    );
    assert!(output.iter().all(|text| !text.contains("Input:")));
    assert!(backend.terminal_commands.is_empty());
}

// 알 수 없는 명령·스레드나 문자열이 아닌 입력은 터미널 안내로 위장하지 못한다.
#[test]
fn terminal_interactions_reject_invalid_targets_and_input() {
    for patch in [
        json!({"itemId":"other"}),
        json!({"threadId":"other"}),
        json!({"stdin":12}),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"command-a","processId":"7","stdin":""});
        params
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"command-a","type":"commandExecution","command":"test"}}}),
            json!({"method":"item/commandExecution/terminalInteraction","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        backend.poll_event().unwrap();
        backend.poll_event().unwrap();
        assert!(backend.poll_event().is_err());
    }
}

// 서버가 보고한 시간만 결과별로 보존하고 계획 종료·시간 안내·턴 종료 순서를 유지한다.
#[test]
fn turn_duration_notice_precedes_terminal_outcome_once() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};

    for status in ["completed", "failed", "interrupted"] {
        for duration in [None, Some(json!(null)), Some(json!(0)), Some(json!(62345))] {
            let session_id = session(1);
            let mut completed = json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"actual failure"}}}});
            if let Some(duration) = &duration {
                completed["params"]["turn"]["durationMs"] = duration.clone();
            }
            let (mut backend, _) = backend([
                thread_start_response(2, "thread-a"),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"method":"turn/plan/updated","params":{"threadId":"thread-a","turnId":"turn-a","plan":[{"step":"Verify","status":"inProgress"}]}}),
                completed.clone(),
                completed,
            ]);
            backend
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: UserInput::from("inspect"),
                })
                .unwrap();
            let mut events = Vec::new();
            while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
                events.push(event);
            }
            let notices = events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| {
                    if let BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(text),
                    } = event
                    {
                        ActivityNotice::from_snapshot(text).map(|notice| (index, *activity, notice))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let expected = duration.as_ref().and_then(|value| value.as_u64());
            assert_eq!(notices.len(), usize::from(expected.is_some()));
            if let Some(ms) = expected {
                let (index, activity, notice) = &notices[0];
                assert_eq!(notice.title, format!("Turn {status}"));
                assert_eq!(
                    notice.level,
                    if status == "failed" {
                        NoticeLevel::Warning
                    } else {
                        NoticeLevel::Info
                    }
                );
                assert!(
                    notice
                        .message
                        .contains(&format!("({ms} ms, reported by Codex)"))
                );
                assert!(
                    notice
                        .message
                        .contains(if ms == 0 { "0.000s" } else { "1m 2.345s" })
                );
                assert!(
                    events[..index - 1]
                        .iter()
                        .any(|event| matches!(event, BackendEvent::ActivityFinished { .. }))
                );
                assert!(
                    matches!(&events[index + 1], BackendEvent::ActivityFinished { activity: finished, outcome: ActivityOutcome::Completed } if finished == activity)
                );
            }
            match (status, events.last().unwrap()) {
                ("completed", BackendEvent::ResumableTurnFinished { .. }) => {},
                (
                    "failed",
                    BackendEvent::TurnFinished {
                        outcome: TurnOutcome::Failed(failure),
                        ..
                    },
                ) => assert_eq!(failure.message(), "actual failure"),
                (
                    "interrupted",
                    BackendEvent::TurnFinished {
                        outcome: TurnOutcome::Interrupted,
                        ..
                    },
                ) => {},
                other => panic!("wrong terminal outcome: {other:?}"),
            }
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        BackendEvent::ResumableTurnFinished { .. }
                            | BackendEvent::TurnFinished { .. }
                    ))
                    .count(),
                1
            );
        }
    }
}

// 음수·문자열·소수·int64 첫 초과 시간은 종료 상태를 바꾸기 전에 거절한다.
#[test]
fn turn_duration_rejects_invalid_protocol_values() {
    for duration in [
        json!(-1),
        json!("12"),
        json!(1.5),
        json!(9223372036854775808u64),
    ] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed","durationMs":duration}}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        assert_eq!(
            backend.poll_event().unwrap_err().kind(),
            BackendFailureKind::Protocol
        );
        assert!(!backend.wire_turns["turn-a"].finished);
    }
}

// 리뷰 모드의 두 경계를 독립 문서로 보존하며 빈 종료 본문을 통과 판정으로 바꾸지 않는다.
#[test]
fn review_boundaries_preserve_markdown_and_empty_or_unknown_content() {
    use yo_core::{ActivityDocument, ActivityUpdate};
    for (kind, title) in [
        ("enteredReviewMode", "Review started"),
        ("exitedReviewMode", "Review ended"),
    ] {
        for review in [
            json!(
                "Review **working tree**\n\n[P1] Preserve errors\n\n```rust\nreturn Err(error);\n```"
            ),
            json!(""),
            json!({"future":"retained"}),
        ] {
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            let item = json!({"id":"review-a","type":kind,"review":review});
            let (backend, _) = backend([
                thread_start_response(2, "thread-a"),
                json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
                json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
                json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
            ]);
            let mut runtime = AgentRuntime::new(backend);
            runtime
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: active_turn,
                        input: "review".into(),
                    },
                    submission(1),
                )
                .unwrap();
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityStarted {
                    activity: activity(active_turn, 1),
                    kind: ActivityKind::ModelWork
                })
            );
            for _ in 0..2 {
                let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                    activity: observed,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                }) = runtime.poll_event().unwrap()
                else {
                    panic!("review document missing");
                };
                assert_eq!(observed, activity(active_turn, 1));
                if let Some(markdown) = review.as_str() {
                    let document = ActivityDocument::from_snapshot(&snapshot).unwrap();
                    assert_eq!(document.title, title);
                    assert_eq!(document.markdown, markdown);
                } else {
                    assert!(ActivityDocument::from_snapshot(&snapshot).is_none());
                    assert!(snapshot.contains("retained"));
                }
            }
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityFinished {
                    activity: activity(active_turn, 1),
                    outcome: ActivityOutcome::Completed
                })
            );
            assert!(matches!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::TurnFinished { .. })
            ));
        }
    }
}

// 에이전트 알림과 대기는 동일 ID의 시작·완료를 한 Activity로 묶고 요청 시간의 정밀도를 지킨다.
#[test]
fn agent_notifications_and_sleep_preserve_identity_and_reported_values() {
    use yo_core::{ActivityNotice, ActivityUpdate, NoticeLevel};
    let mut cases = Vec::new();
    for kind in ["started", "interacted", "interrupted", "completed"] {
        cases.push((json!({"id":"item-a","type":"subAgentActivity","kind":kind,"agentPath":"/root/검토","agentThreadId":"child-a","future":"retained"}), ActivityKind::ModelWork));
    }
    for duration in [json!(0), json!(1500), json!(u64::MAX), json!(-1)] {
        cases.push((
            json!({"id":"item-a","type":"sleep","durationMs":duration,"future":"retained"}),
            ActivityKind::ToolCall,
        ));
    }
    for (item, kind) in cases {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "observe".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind
            })
        );
        let mut previous = None;
        for _ in 0..2 {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("snapshot missing or duplicate start");
            };
            assert_eq!(observed, activity(active_turn, 1));
            if let Some(previous) = &previous {
                assert_eq!(&snapshot, previous);
            }
            if item["type"] == "subAgentActivity" {
                let notice = ActivityNotice::from_snapshot(&snapshot).unwrap();
                assert_eq!(notice.level, NoticeLevel::Info);
                assert_eq!(
                    notice.title,
                    format!("Agent activity · {}", item["kind"].as_str().unwrap())
                );
                for text in ["/root/검토", "child-a", "retained"] {
                    assert!(notice.message.contains(text));
                }
            } else if let Some(duration) = item["durationMs"].as_u64() {
                let output = ToolOutput::from_snapshot(&snapshot).unwrap();
                assert_eq!(output.tool, "clock.sleep");
                assert_eq!(output.arguments, Some(json!({"durationMs":duration})));
                assert!(
                    output
                        .plain_text
                        .contains(&format!("Requested duration: {duration} ms"))
                );
                assert!(output.plain_text.contains("retained"));
                assert_eq!(output.content_blocks().next().unwrap()["source"], item);
            } else {
                assert!(ToolOutput::from_snapshot(&snapshot).is_none());
                assert!(snapshot.contains("-1") && snapshot.contains("retained"));
            }
            previous = Some(snapshot);
        }
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: activity(active_turn, 1),
                outcome: ActivityOutcome::Completed
            })
        );
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Completed,
                ..
            })
        ));
    }
}

// 훅 문맥을 사용자 제출과 구분하고 여러 fragment의 순서·원문·fence 및 미지 필드를 보존한다.
#[test]
fn hook_context_is_one_literal_document_with_original_fragment_order() {
    use yo_core::{ActivityDocument, ActivityUpdate};
    for fragments in [
        json!([{"hookRunId":"hook-a","text":"First **context**\n````\n![literal](data:image/png;base64,AAAA)","future":"fragment-kept"},{"hookRunId":"hook-b","text":"Second context"}]),
        json!([]),
        json!([{"hookRunId":"hook-a","text":42}]),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let item = json!({"id":"hook-item","type":"hookPrompt","fragments":fragments,"future":"item-kept"});
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "inspect context".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind: ActivityKind::ModelWork
            })
        );
        for _ in 0..2 {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("hook document missing or input was duplicated");
            };
            assert_eq!(observed, activity(active_turn, 1));
            if fragments[0]
                .get("text")
                .is_some_and(|text| !text.is_string())
            {
                assert!(ActivityDocument::from_snapshot(&snapshot).is_none());
                assert!(snapshot.contains("42") && snapshot.contains("item-kept"));
            } else {
                let doc = ActivityDocument::from_snapshot(&snapshot).unwrap();
                assert_eq!(doc.title, "Hook context");
                assert!(doc.markdown.contains("item-kept"));
                if fragments.as_array().unwrap().is_empty() {
                    assert!(doc.markdown.contains("No hook context fragments reported"));
                } else {
                    assert!(doc.markdown.starts_with("`````text\n"));
                    assert!(doc.markdown.ends_with("\n`````"));
                    for fragment in fragments.as_array().unwrap() {
                        assert!(doc.markdown.contains(fragment["text"].as_str().unwrap()));
                    }
                    assert!(
                        doc.markdown.find("Hook run: hook-a").unwrap()
                            < doc.markdown.find("Hook run: hook-b").unwrap()
                    );
                    assert!(doc.markdown.contains("fragment-kept"));
                }
            }
        }
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: activity(active_turn, 1),
                outcome: ActivityOutcome::Completed
            })
        );
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished { .. })
        ));
    }
}

// Codex native skill path 재로딩 없이 공통 host가 검증한 전체 지침을 text 입력으로 전달한다.
// 시작과 steer 모두 같은 snapshot을 쓰며 원래 `$name`만 보내지 않는다.
#[test]
fn resolved_skill_snapshot_reaches_codex_start_and_steer_requests() {
    use yo_core::{InputReference, ResolvedSkill, SkillReference, SkillReferenceScope};
    let reference = SkillReference::new(
        "review",
        "host",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let input = UserInput::with_references(
        "use $review",
        vec![InputReference::skill(4..11, reference.clone())],
    )
    .unwrap()
    .with_resolved_skill(
        ResolvedSkill::new(reference, "# Review\nVerify the exact implementation.").unwrap(),
    )
    .unwrap();
    let expected = input.model_input().to_owned();
    let session_id = session(1);
    let active = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":4,"result":{"turnId":"turn-a"}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active,
            input: input.clone(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::SteerTurn {
            turn: active,
            input,
        })
        .unwrap();
    let sent = sent.0.borrow();
    for method in ["turn/start", "turn/steer"] {
        let request = sent
            .iter()
            .find(|request| request["method"] == method)
            .unwrap();
        assert_eq!(
            request["params"]["input"],
            json!([{"type":"text", "text": expected}])
        );
    }
}

// immutable PNG occurrence와 host-resolved skill trailer가 start와 steer 모두에서 정확한
// text/image 순서와 마지막 지침 위치를 유지하며, 표시 metadata나 별도 caption을 만들지 않습니다.
#[test]
fn immutable_png_projection_preserves_repeated_occurrences_on_start_and_steer() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use yo_core::{InputReference, ResolvedSkill, SkillReference, SkillReferenceScope};

    let png = STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==")
        .unwrap();
    let snapshot = InputImageSnapshot::new(1, 1, png.clone()).unwrap();
    let text = "before [image] middle [image] use $review after";
    let reference = SkillReference::new(
        "review",
        "host",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let input =
        UserInput::with_references(text, vec![InputReference::skill(34..41, reference.clone())])
            .unwrap()
            .with_images(vec![
                InputImage::new(7..14, 70, snapshot.clone()).unwrap(),
                InputImage::new(22..29, 70, snapshot).unwrap(),
            ])
            .unwrap()
            .with_resolved_skill(
                ResolvedSkill::new(reference, "# Review\nVerify the exact implementation.")
                    .unwrap(),
            )
            .unwrap();
    let expected_url = format!("data:image/png;base64,{}", STANDARD.encode(png));
    let parts = super::super::super::project_input(&input).unwrap();
    let skill_snapshot = json!({
        "name": "review",
        "source": "/skills/review/SKILL.md",
        "instructions": "# Review\nVerify the exact implementation."
    });
    let expected_tail = format!(
        " use $review after\n\nExplicit skill instructions (yo.skill-instructions/v1):\n{}",
        skill_snapshot
    );
    assert_eq!(
        parts,
        vec![
            json!({"type":"text", "text":"before "}),
            json!({"type":"image", "url": expected_url.clone()}),
            json!({"type":"text", "text":" middle "}),
            json!({"type":"image", "url": expected_url}),
            json!({"type":"text", "text": expected_tail}),
        ]
    );
    assert!(matches!(
        input.model_parts()[1],
        ModelInputPart::Image { .. }
    ));

    let session_id = session(1);
    let active = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({
            "id": 3,
            "result": {
                "data": [{"model": "gpt-test", "inputModalities": ["text", "image"]}],
                "nextCursor": null
            }
        }),
        json!({"id":4,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":5,"result":{"turnId":"turn-a"}}),
    ]);
    backend.image_wire_supported = true;
    backend.selected_model = Some("gpt-test".to_owned());
    backend.image_capability = ImageInputCapability::Supported {
        maximum_occurrences: 16,
        maximum_image_bytes: InputImageSnapshot::MAX_BYTES as u64,
        maximum_input_bytes: InputImageSnapshot::MAX_BYTES as u64,
    };
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active,
            input: input.clone(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::SteerTurn {
            turn: active,
            input,
        })
        .unwrap();
    let sent = sent.0.borrow();
    for method in ["turn/start", "turn/steer"] {
        let request = sent
            .iter()
            .find(|request| request["method"] == method)
            .unwrap();
        assert_eq!(request["params"]["input"], json!(parts.clone()));
    }
}
