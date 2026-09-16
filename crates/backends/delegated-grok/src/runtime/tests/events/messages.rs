use std::iter;

use yo_core::ToolOutput;

use super::super::*;

// messageId가 없는 agent/thought stream은 ToolCall과 승인 경계를 넘어서 같은 Activity를
// 재사용하지 않고, 각 경계 뒤에 새 Activity를 시작합니다.
#[test]
fn splits_unidentified_agent_and_thought_chunks_at_tool_and_approval_boundaries() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        text_update("agent_message_chunk", "before tool"),
        tool_call("tool-a", "in_progress", Some("Run cargo test")),
        text_update("agent_message_chunk", "after tool"),
        text_update("agent_thought_chunk", "before approval"),
        permission_request("permission-a", Some("Run cargo test")),
        text_update("agent_thought_chunk", "after approval"),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let agent_before = expect_activity_started(&mut backend, ActivityKind::AgentMessage);
    expect_activity_update(&mut backend, agent_before);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == agent_before
    ));

    let tool_activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
    expect_activity_update(&mut backend, tool_activity);

    let agent_after = expect_activity_started(&mut backend, ActivityKind::AgentMessage);
    assert_ne!(agent_after, agent_before);
    expect_activity_update(&mut backend, agent_after);

    let thought_before = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    expect_activity_update(&mut backend, thought_before);

    for activity in [agent_after, thought_before] {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished { activity: observed, .. })
                if observed == activity
        ));
    }
    let (approval_activity, request_id) = match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) => (activity, request_id),
        other => panic!("permission must start an approval request, got {other:?}"),
    };
    expect_activity_update(&mut backend, approval_activity);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: yo_core::ActivityRequestRef::new(approval_activity, request_id),
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == approval_activity
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted {
            kind: ActivityKind::ApprovalResponse { .. },
            ..
        })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { update:yo_core::ActivityUpdate::TextSnapshot(text),.. })
            if text.contains("Response sent to agent.")
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { .. })
    ));

    let thought_after = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    assert_ne!(thought_after, thought_before);
    expect_activity_update(&mut backend, thought_after);
}

// ACP 계획은 전체 목록을 교체하고 도구 호출 중에도 같은 Activity를 유지하며 빈 목록도 전달한다.
#[test]
fn grok_plan_replaces_steps_across_tools_and_closes_with_turn() {
    use yo_core::{ActivityPlan, ActivityUpdate, PlanStepStatus};
    for stop in ["end_turn", "cancelled"] {
        let session_id = session(1);
        let entry = |content: &str, status: &str| json!({"content":content,"priority":"high","status":status});
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update(
                "plan",
                json!({"entries":[entry("Read", "pending"), entry("Test", "pending")]}),
            ),
            tool_call("tool-a", "completed", Some("Read")),
            session_update("plan", json!({"entries":[entry("Revised", "in_progress")]})),
            session_update(
                "plan",
                json!({"entries": if stop == "cancelled" { vec![entry("Revised", "in_progress")] } else { vec![] }}),
            ),
            response(4, json!({"stopReason":stop})),
            session_update("plan", json!({"entries":[entry("Next", "completed")]})),
            response(5, json!({"stopReason":"end_turn"})),
        ]);
        create_session(&mut backend, session_id);
        let mut previous = None;
        for number in [1, 2] {
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, number),
                    input: UserInput::from("plan"),
                })
                .unwrap();
            let mut plans = Vec::new();
            let mut finished = Vec::new();
            let mut ended = false;
            for _ in 0..64 {
                match backend.poll_event().unwrap() {
                    BackendPoll::Event(BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(text),
                    }) => {
                        if let Some(plan) = ActivityPlan::from_snapshot(&text) {
                            plans.push((activity, plan));
                        }
                    },
                    BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                        finished.push((activity, outcome))
                    },
                    BackendPoll::Event(
                        BackendEvent::ResumableTurnFinished { .. }
                        | BackendEvent::TurnFinished { .. },
                    ) => {
                        ended = true;
                        break;
                    },
                    BackendPoll::Event(_) | BackendPoll::Pending => {},
                    other => panic!("unexpected plan event: {other:?}"),
                }
            }
            assert!(ended);
            let id = plans[0].0;
            assert_ne!(Some(id), previous);
            assert!(plans.iter().all(|(activity, _)| *activity == id));
            if number == 1 {
                assert_eq!(plans.len(), 3);
                assert_eq!(plans[0].1.steps.len(), 2);
                assert_eq!(plans[1].1.steps.len(), 1);
                assert_eq!(plans[1].1.steps[0].text, "[high] Revised");
                assert_eq!(plans[1].1.steps[0].status, PlanStepStatus::InProgress);
                assert_eq!(plans[2].1.steps.is_empty(), stop == "end_turn");
                if stop == "cancelled" {
                    assert_eq!(plans[2].1.steps[0].status, PlanStepStatus::InProgress);
                }
            } else {
                assert_eq!(plans[0].1.steps[0].status, PlanStepStatus::Completed);
            }
            let outcomes: Vec<_> = finished
                .iter()
                .filter(|(activity, _)| *activity == id)
                .collect();
            assert_eq!(outcomes.len(), 1);
            assert!(
                matches!(&outcomes[0].1, ActivityOutcome::Interrupted)
                    == (number == 1 && stop == "cancelled")
            );
            previous = Some(id);
        }
    }
}

// 잘못된 계획과 표시 한도 초과는 Activity를 열기 전에 거부해 부분 계획을 남기지 않는다.
#[test]
fn grok_plan_rejects_invalid_entries_before_publishing() {
    use yo_core::{ActivityPlan, ActivityUpdate, PlanStep, PlanStepStatus};
    let overhead = ActivityPlan {
        explanation: None,
        steps: vec![PlanStep {
            text: "[medium] ".into(),
            status: PlanStepStatus::Pending,
        }],
    }
    .to_snapshot()
    .unwrap()
    .len();
    let limit = ToolOutput::MAX_SNAPSHOT_BYTES - overhead;
    for update in [
        json!({}),
        json!({"entries":[{"content":"x","priority":"urgent","status":"pending"}]}),
        json!({"entries":[{"content":"x","priority":"low","status":"unknown"}]}),
        json!({"entries":[{"content":"x".repeat(limit + 1),"priority":"medium","status":"pending"}]}),
    ] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update("plan", update),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("plan"),
            })
            .unwrap();
        assert_eq!(
            backend.poll_event().unwrap_err().kind(),
            BackendFailureKind::Protocol
        );
        assert!(backend.messages.is_empty());
        assert!(backend.pending_events.is_empty());
    }
    let session_id = session(1);
    let (mut backend, _) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update(
            "plan",
            json!({"entries":[{"content":"x".repeat(limit),"priority":"medium","status":"pending"}]}),
        ),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("plan"),
        })
        .unwrap();
    let activity = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity: observed,
        update: ActivityUpdate::TextSnapshot(text),
    }) = backend.poll_event().unwrap()
    else {
        panic!("bounded plan missing")
    };
    assert_eq!(observed, activity);
    assert_eq!(text.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert!(ActivityPlan::from_snapshot(&text).is_some());
}

// 텍스트 사이 이미지·리소스·미지 콘텐츠를 버리지 않고 원래 순서의 독립 메시지로 전달한다.
#[test]
fn answer_content_preserves_payload_and_splits_text_stream_in_order() {
    use yo_core::{ActivityUpdate, MessageContent};
    for message_id in [None, Some("answer-a")] {
        for block in [
            json!({"type":"image","mimeType":"image/png","data":"AA==","_meta":{"extra":1}}),
            json!({"type":"resource","resource":{"uri":"file:///remote/main.rs","text":"fn main() {}"}}),
            json!({"type":"future","payload":[1,2,3]}),
        ] {
            let update = |content: Value| {
                let mut fields = json!({"content":content});
                if let Some(id) = message_id {
                    fields["messageId"] = id.into();
                }
                session_update("agent_message_chunk", fields)
            };
            let session_id = session(1);
            let (mut backend, _) = backend([
                response(3, json!({"sessionId":"grok-session-a"})),
                update(json!({"type":"text","text":"before"})),
                update(block.clone()),
                update(json!({"type":"text","text":"after"})),
                response(4, json!({"stopReason":"end_turn"})),
            ]);
            create_session(&mut backend, session_id);
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: "show content".into(),
                })
                .unwrap();
            let mut opened = None;
            let mut sources = Vec::new();
            let mut completed = false;
            for _ in 0..32 {
                match backend.poll_event().unwrap() {
                    BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
                        assert_eq!(kind, ActivityKind::AgentMessage);
                        assert!(opened.replace(activity).is_none());
                    },
                    BackendPoll::Event(BackendEvent::ActivityUpdated { activity, update }) => {
                        assert_eq!(opened, Some(activity));
                        let (ActivityUpdate::TextDelta(text) | ActivityUpdate::TextSnapshot(text)) =
                            update;
                        sources.push(text);
                    },
                    BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                        assert_eq!(opened.take(), Some(activity));
                        assert_eq!(outcome, ActivityOutcome::Completed);
                    },
                    BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                        completed = true;
                        break;
                    },
                    _ => {},
                }
            }
            assert!(completed);
            assert!(opened.is_none());
            assert_eq!(sources.len(), 3);
            assert_eq!(sources[0], "before");
            assert_eq!(
                MessageContent::from_snapshot(&sources[1]).unwrap().block,
                block
            );
            assert_eq!(sources[2], "after");
        }
    }
}

// 추론 청크는 누적 snapshot을 내보내고 비텍스트 경계 뒤에는 새 Activity로 원문 순서를 보존한다.
#[test]
fn reasoning_stream_preserves_accumulation_and_content_boundaries() {
    use yo_core::{ActivityReasoning, ActivityUpdate};
    for message_id in [None, Some("thought-a")] {
        let block =
            json!({"type":"future", "value":"![literal](data:image/png;base64,AA==)", "extra":7});
        let updates = [
            json!("first "),
            json!("한글"),
            block.clone(),
            json!("after"),
        ]
        .map(|content| {
            let mut update = text_update("agent_thought_chunk", "");
            update["params"]["update"]["content"] = if let Some(text) = content.as_str() {
                json!({"type":"text", "text":text})
            } else {
                content
            };
            if let Some(id) = message_id {
                update["params"]["update"]["messageId"] = json!(id);
            }
            update
        });
        let messages = iter::once(response(3, json!({"sessionId":"grok-session-a"})))
            .chain(updates)
            .chain(iter::once(response(4, json!({"stopReason":"end_turn"}))));
        let (mut backend, _) = backend(messages);
        let session_id = session(1);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        let mut active = None;
        let mut started = Vec::new();
        let mut snapshots = Vec::new();
        for _ in 0..20 {
            match backend.poll_event().unwrap() {
                BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
                    assert_eq!(kind, ActivityKind::ModelWork);
                    assert!(active.replace(activity).is_none());
                    started.push(activity);
                },
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(source),
                }) => {
                    assert_eq!(active, Some(activity));
                    snapshots.push(ActivityReasoning::from_snapshot(&source).unwrap().content);
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                    assert_eq!(active.take(), Some(activity));
                    assert!(matches!(outcome, ActivityOutcome::Completed));
                },
                BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => break,
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert!(active.is_none());
        assert_eq!(started.len(), 3);
        assert!(started.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(
            snapshots,
            vec![json!("first "), json!("first 한글"), block, json!("after")]
        );
    }
}

// 누적 추론의 크기 초과는 일반 텍스트로 우회하지 않고 이미 전달한 snapshot 다음에 명시적으로
// 실패한다.
#[test]
fn reasoning_stream_rejects_oversized_continuation_without_literal_fallback() {
    let messages = [
        response(3, json!({"sessionId":"grok-session-a"})),
        text_update("agent_thought_chunk", "retained"),
        text_update(
            "agent_thought_chunk",
            &"x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES),
        ),
    ];
    let (mut backend, _) = backend(messages);
    let session_id = session(1);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let activity = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    expect_activity_update(&mut backend, activity);
    assert!(backend.poll_event().is_err());
}
