use yo_core::ToolOutput;

use super::super::*;

// 서로 다른 update로 온 tool name과 rawInput은 하나의 ToolCall identity로 누적하고,
// 승인 뒤의 반복 name은 이를 축소하지 않으며 결과는 별도 ToolResult로 투영합니다.
#[test]
fn keeps_tool_call_identity_when_tool_result_follows_an_approval_round_trip() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let mut call = tool_call("tool-a", "in_progress", Some(""));
    call["params"]["update"]["name"] = json!("terminal");
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        call,
        tool_raw_input_update("tool-a", json!({ "command": "cargo test" })),
        permission_request("permission-a", Some("Run cargo test")),
        tool_result("tool-a", "all tests passed"),
        response(4, json!({ "stopReason": "end_turn" })),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("run tests"),
        })
        .unwrap();

    let tool_activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(identity),
        }) if activity == tool_activity && identity == "terminal"
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(identity),
        }) if activity == tool_activity && identity == "terminal: {\"command\":\"cargo test\"}"
    ));

    let (approval_activity, request_id) = match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) => (activity, request_id),
        other => panic!("permission must start an approval request, got {other:?}"),
    };
    expect_activity_update(&mut backend, approval_activity);
    let request = yo_core::ActivityRequestRef::new(approval_activity, request_id);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();

    let approval_finished = backend.poll_event().unwrap();
    assert!(matches!(
        approval_finished,
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == approval_activity
    ));
    let response_activity = match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind:
                ActivityKind::ApprovalResponse {
                    request_id: observed,
                },
        }) => {
            assert_eq!(observed, request_id);
            activity
        },
        other => panic!("approval response must start an Activity, got {other:?}"),
    };
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { activity, update:yo_core::ActivityUpdate::TextSnapshot(text) })
            if activity == response_activity && text.contains("Allow this operation once.")
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == response_activity
    ));

    let result_activity = expect_activity_started(&mut backend, ActivityKind::ToolResult);
    assert_ne!(result_activity, tool_activity);
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity,
        update: yo_core::ActivityUpdate::TextSnapshot(result),
    }) = backend.poll_event().unwrap()
    else {
        panic!("tool content must update the ToolResult Activity");
    };
    assert_eq!(activity, result_activity);
    let output = ToolOutput::from_snapshot(&result).unwrap();
    assert_eq!(output.tool, "terminal");
    assert_eq!(output.arguments, Some(json!({"command":"cargo test"})));
    assert_eq!(
        output.content_items,
        Some(json!([{"type":"text","text":"all tests passed"}]))
    );
    assert!(output.plain_text.contains("all tests passed"));
    for activity in [result_activity, tool_activity] {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity: observed,
                outcome: ActivityOutcome::Completed,
            }) if observed == activity
        ));
    }
}

// name과 rawInput이 어느 순서로 나뉘어 도착해도 기존 field를 잃지 않고 같은 ToolCall
// Activity의 `name: input` snapshot으로 합쳐 부분 update 순서에 의존하지 않습니다.
#[test]
fn merges_split_tool_identity_in_either_arrival_order() {
    for (mut initial, mut later, first_snapshot) in [
        (
            json!({ "name": "terminal" }),
            json!({ "rawInput": { "command": "cargo test" } }),
            "terminal",
        ),
        (
            json!({ "rawInput": { "command": "cargo test" } }),
            json!({ "name": "terminal" }),
            "{\"command\":\"cargo test\"}",
        ),
    ] {
        for update in [&mut initial, &mut later] {
            update["toolCallId"] = json!("tool-a");
            update["status"] = json!("in_progress");
        }
        let session_id = session(1);
        let (mut backend, _) = backend([
            response(3, json!({ "sessionId": "grok-session-a" })),
            session_update("tool_call", initial),
            session_update("tool_call_update", later),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();

        let activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
        for expected in [first_snapshot, "terminal: {\"command\":\"cargo test\"}"] {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity: observed,
                    update: yo_core::ActivityUpdate::TextSnapshot(snapshot),
                }) if observed == activity && snapshot == expected
            ));
        }
    }
}

// 출력이 빈 완료 호출도 상관관계가 있는 ToolResult를 남기며, 그 ACP ToolCallId는 다음
// Turn까지 tombstone으로 유지되어 재사용을 새 Activity로 잘못 열지 않습니다.
#[test]
fn rejects_a_completed_tool_call_id_reused_in_a_later_turn() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let second_turn = turn(session_id, 2);
    let completed_tool = || {
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "grok-session-a",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-a",
                    "status": "completed"
                }
            }
        })
    };
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        completed_tool(),
        response(4, json!({ "stopReason": "end_turn" })),
        completed_tool(),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("first"),
        })
        .unwrap();

    let tool_activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
    expect_activity_update(&mut backend, tool_activity);
    let result_activity = expect_activity_started(&mut backend, ActivityKind::ToolResult);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(result),
        }) if activity == result_activity
            && ToolOutput::from_snapshot(&result).is_some_and(|output|
                output.tool == "tool-a" && output.content_items.is_none() && output.result.is_none())
    ));
    for activity in [result_activity, tool_activity] {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity: observed,
                ..
            }) if observed == activity
        ));
    }
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. })
    ));

    backend
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second"),
        })
        .unwrap();
    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("duplicate Grok ACP tool call"));
}

// ACP 부분 갱신은 생략한 결과·인자를 유지하고 명시적인 빈 content만 지우며 원래 콘텐츠를 보존한다.
#[test]
fn grok_tool_output_preserves_partial_fields_and_explicit_empty_content() {
    use yo_core::ActivityUpdate;
    let session_id = session(1);
    let image = json!({"type":"image","mimeType":"image/png","data":"AA=="});
    let diff = json!({"type":"diff","path":"/work/file","oldText":"old","newText":"new"});
    let (mut backend, _) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update(
            "tool_call",
            json!({"toolCallId":"tool-a","name":"inspect","status":"in_progress","rawInput":{"path":"a"},"content":[{"type":"content","content":{"type":"text","text":"partial"}},{"type":"content","content":image},diff]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","rawOutput":{"matches":3},"_meta":{"source":"fixture"}}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","content":[]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","rawInput":{"path":"b"}}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","status":"failed"}),
        ),
        response(4, json!({"stopReason":"end_turn"})),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let mut outputs = Vec::new();
    let mut failed = Vec::new();
    let mut released = false;
    let mut ended = false;
    for _ in 0..64 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) => {
                if let Some(output) = ToolOutput::from_snapshot(&text) {
                    outputs.push((activity, output));
                }
            },
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(_),
            }) => {
                failed.push(activity);
                released = backend
                    .tools
                    .get("tool-a")
                    .is_some_and(|binding| binding.output == json!({}));
            },
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                ended = true;
                break;
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            other => panic!("unexpected tool output: {other:?}"),
        }
    }
    assert!(ended);
    assert_eq!(outputs.len(), 4);
    let id = outputs[0].0;
    assert!(
        outputs
            .iter()
            .all(|(activity, output)| *activity == id && output.tool == "inspect")
    );
    assert_eq!(outputs[0].1.arguments, Some(json!({"path":"a"})));
    assert_eq!(outputs[0].1.content_items.as_ref().unwrap()[1], image);
    assert_eq!(
        outputs[0].1.content_items.as_ref().unwrap()[2]["source"],
        diff
    );
    assert!(
        outputs[0].1.content_items.as_ref().unwrap()[2]["text"]
            .as_str()
            .unwrap()
            .contains("-old\n")
    );
    assert_eq!(outputs[1].1.content_items, outputs[0].1.content_items);
    assert_eq!(
        outputs[1].1.result,
        Some(json!({"rawOutput":{"matches":3},"_meta":{"source":"fixture"}}))
    );
    assert_eq!(outputs[2].1.content_items, Some(json!([])));
    assert!(!outputs[2].1.plain_text.contains("partial"));
    assert_eq!(outputs[3].1.arguments, Some(json!({"path":"b"})));
    assert_eq!(outputs[3].1.result, outputs[1].1.result);
    assert!(failed.contains(&id));
    assert!(outputs.iter().all(|(_, output)| output.error.is_none()));
    assert!(released);
    assert!(backend.tools.is_empty());
}

// ACP 파일 비교는 문맥 hunk·새 파일·개행 없음·경계 크기를 보존하고 미지원 입력은 원문으로 남긴다.
#[test]
fn grok_diff_content_has_context_and_preserves_source_boundaries() {
    use yo_core::ActivityUpdate;
    let old = (0..40).map(|n| format!("line{n}\n")).collect::<String>();
    let new = old
        .replace("line1\n", "changed1\n")
        .replace("line30\n", "changed30\n");
    for (index, (old, new)) in [
        (json!(old), new),
        (Value::Null, "created".into()),
        (json!("same"), "same".into()),
        (json!(42), "invalid".into()),
        (Value::Null, "x".repeat(256 * 1024)),
        (Value::Null, "x".repeat(256 * 1024 + 1)),
    ]
    .into_iter()
    .enumerate()
    {
        let source = json!({"type":"diff","path":"/work/file\nname","oldText":old,"newText":new,"_meta":{"kept":true}});
        let session_id = session(1);
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update(
                "tool_call",
                json!({"toolCallId":"tool-a","status":"completed","content":[source.clone()]}),
            ),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("compare"),
            })
            .unwrap();
        let mut observed = None;
        for _ in 0..8 {
            if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
                && let Some(output) = ToolOutput::from_snapshot(&text)
            {
                observed = Some(output);
                break;
            }
        }
        let output = observed.expect("typed result");
        let item = &output.content_items.as_ref().unwrap()[0];
        if index == 3 || index == 5 {
            assert_eq!(item, &source);
            continue;
        }
        assert_eq!(item["source"], source);
        let text = item["text"].as_str().unwrap();
        if index == 0 {
            assert_eq!(text.matches("@@ -").count(), 2);
            assert!(text.contains("-line1\n+changed1\n"));
            assert!(!text.contains("line15"));
            assert!(text.starts_with("--- \"/work/file\\nname\"\n"));
        } else if index == 1 {
            assert!(text.starts_with("--- /dev/null\n"));
            assert!(text.contains("+created\n\\ No newline at end of file"));
        } else if index == 2 {
            assert!(text.is_empty());
        }
    }
}

// 파일 변경 본문은 같은 호출 ID에서 여러 파일·부분 갱신·명시적 삭제를 반영하고 실패 상태를
// 유지한다.
#[test]
fn grok_file_change_body_tracks_reported_diffs_and_clearing() {
    use yo_core::ActivityUpdate;
    let session_id = session(1);
    let first = json!({"type":"diff","path":"a.rs","oldText":"old\n","newText":"new\n"});
    let second = json!({"type":"diff","path":"b\n.rs","oldText":null,"newText":"created\n"});
    let (mut backend, _) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update(
            "tool_call",
            json!({"toolCallId":"edit-a","kind":"edit","title":"Patch files","status":"in_progress","content":[first,second]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"edit-a","rawOutput":{"note":"partial"}}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"edit-a","content":[]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"edit-a","status":"failed"}),
        ),
        response(4, json!({"stopReason":"end_turn"})),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("edit"),
        })
        .unwrap();
    let mut call = None;
    let mut snapshots = Vec::new();
    let mut failed = false;
    for _ in 0..60 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::FileChange,
            }) => {
                assert!(call.replace(activity).is_none());
            },
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) if Some(activity) == call => snapshots.push(text),
            BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome })
                if Some(activity) == call =>
            {
                failed = matches!(outcome, ActivityOutcome::Failed(_))
            },
            BackendPoll::Event(BackendEvent::TurnFinished { .. }) => break,
            _ => {},
        }
    }
    assert!(failed);
    assert_eq!(snapshots.len(), 4);
    assert!(snapshots[1].starts_with("update: \"a.rs\"\n"));
    assert!(snapshots[1].contains("add: \"b\\n.rs\"\n"));
    assert!(snapshots[1].contains("-old\n+new\n"));
    assert!(snapshots[1].contains("+created\n"));
    assert_eq!(snapshots[1], snapshots[2]);
    assert_eq!(snapshots[0], snapshots[3]);
}
