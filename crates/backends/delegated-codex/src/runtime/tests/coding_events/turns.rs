use super::{super::support::*, *};

// 계획 갱신은 같은 Activity를 교체하고 Turn 완료 전에 종료하며 미완료 단계를 임의로 완료하지
// 않는다.
#[test]
fn updates_one_plan_activity_and_closes_it_before_turn_completion() {
    use yo_core::{ActivityPlan, ActivityUpdate, BackendEvent, BackendPoll, PlanStepStatus};
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let plan = |status| json!({"method":"turn/plan/updated","params":{"threadId":"thread-a","turnId":"turn-a","plan":[{"step":"Inspect","status":"completed"},{"step":"Verify","status":status}]}});
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        plan("pending"),
        plan("inProgress"),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        plan("completed"),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("plan"),
        })
        .unwrap();
    let mut events = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        events.push(event);
    }
    assert_eq!(events.len(), 5);
    for (index, status) in [
        (1, PlanStepStatus::Pending),
        (2, PlanStepStatus::InProgress),
    ] {
        let BackendEvent::ActivityUpdated {
            activity: owner,
            update: ActivityUpdate::TextSnapshot(text),
        } = &events[index]
        else {
            panic!("missing plan snapshot");
        };
        assert_eq!(*owner, activity(active_turn, 1));
        let plan = ActivityPlan::from_snapshot(text).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].text, "Inspect");
        assert_eq!(plan.steps[0].status, PlanStepStatus::Completed);
        assert_eq!(plan.steps[1].text, "Verify");
        assert_eq!(plan.steps[1].status, status);
    }
    assert!(matches!(events[3], BackendEvent::ActivityFinished { .. }));
    assert!(matches!(
        events[4],
        BackendEvent::ResumableTurnFinished { .. }
    ));
}

// 제안 계획은 시작 원문과 delta를 순서대로 합치고 최종 snapshot으로 같은 활동의 본문을 교체한다.
#[test]
fn proposed_plan_streams_documents_and_replaces_with_final_source() {
    use yo_core::{ActivityDocument, ActivityUpdate, BackendEvent, BackendPoll};
    let session_id = session(1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"plan-a","type":"plan","text":"## Draft\n"}}}),
        json!({"method":"item/plan/delta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"plan-a","delta":"- Inspect"}}),
        json!({"method":"item/plan/delta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"plan-a","delta":"\n- Verify"}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"plan-a","type":"plan","text":"## Final\n- Review first"}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("plan"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("missing proposed plan");
    };
    for expected in [
        "## Draft\n",
        "## Draft\n- Inspect",
        "## Draft\n- Inspect\n- Verify",
        "## Final\n- Review first",
    ] {
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: owner,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing document");
        };
        assert_eq!(owner, activity);
        assert_eq!(
            ActivityDocument::from_snapshot(&text),
            Some(ActivityDocument {
                title: "Proposed plan".to_owned(),
                markdown: expected.to_owned()
            })
        );
    }
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed
        })
    );
}

// 제안 계획 delta가 다른 종류의 항목을 바꾸지 않으며 원래 thread 경계를 유지한다.
#[test]
fn proposed_plan_rejects_wrong_item_kind_and_thread() {
    for (kind, thread) in [("agentMessage", "thread-a"), ("plan", "other")] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"plan-a","type":kind,"text":""}}}),
            json!({"method":"item/plan/delta","params":{"threadId":thread,"turnId":"turn-a","itemId":"plan-a","delta":"content"}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("plan"),
            })
            .unwrap();
        backend.poll_event().unwrap();
        if kind == "plan" {
            backend.poll_event().unwrap();
        }
        assert!(backend.poll_event().is_err());
    }
}

// 제공자가 보고한 모델 변경은 순서 있는 안내로 보존하며 완료·실패 결과를 바꾸지 않고 늦은 알림은
// 무시한다.
#[test]
fn model_reroute_preserves_reported_details_and_terminal_outcome() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};
    for status in ["completed", "failed"] {
        let session_id = session(1);
        let reroute = json!({"method":"model/rerouted","params":{"threadId":"thread-a","turnId":"turn-a","fromModel":"requested","toModel":"actual","reason":"highRiskCyberActivity"}});
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            reroute.clone(),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"terminal cause"}}}}),
            reroute,
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
            panic!("missing reroute start")
        };
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: updated,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing reroute content")
        };
        assert_eq!(updated, activity);
        let notice = ActivityNotice::from_snapshot(&text).unwrap();
        assert_eq!(notice.title, "Model rerouted");
        assert_eq!(notice.level, NoticeLevel::Warning);
        assert_eq!(
            notice.message,
            "Codex reported a model change.\nFrom: requested\nTo: actual\nReason: highRiskCyberActivity"
        );
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
            other => panic!("unexpected terminal: {other:?}"),
        }
        assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    }
}

// 잘못된 대상·필드·크기 알림은 안내 activity를 일부 발행하기 전에 거부한다.
#[test]
fn malformed_model_reroute_is_rejected_before_notice_publication() {
    for (field, value) in [
        ("threadId", json!("other-thread")),
        ("turnId", json!("unknown-turn")),
        ("fromModel", json!(null)),
        ("toModel", json!(42)),
        ("reason", json!(null)),
        ("reason", json!("x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES))),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","fromModel":"requested","toModel":"actual","reason":"highRiskCyberActivity"});
        params[field] = value;
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"model/rerouted","params":params}),
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
        assert!(backend.pending_events.is_empty());
    }
}

// 안내 스냅샷의 정확한 한도는 발행하고 첫 초과 바이트는 activity를 만들기 전에 거부한다.
#[test]
fn model_reroute_notice_accepts_exact_snapshot_limit_only() {
    use yo_core::{ActivityNotice, BackendEvent, BackendPoll, NoticeLevel};
    let overhead = ActivityNotice {
        title: "Model rerouted".to_owned(),
        message: "Codex reported a model change.\nFrom: requested\nTo: actual\nReason: ".to_owned(),
        level: NoticeLevel::Warning,
    }
    .to_snapshot()
    .unwrap()
    .len();
    for extra in [0, 1] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"model/rerouted","params":{"threadId":"thread-a","turnId":"turn-a","fromModel":"requested","toModel":"actual","reason":"x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead + extra)}}),
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
        if extra == 0 {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted { .. })
            ));
        } else {
            assert_eq!(
                backend.poll_event().unwrap_err().kind(),
                BackendFailureKind::Protocol
            );
            assert!(backend.pending_events.is_empty());
        }
    }
}

// 턴 누적 diff는 같은 activity를 교체하고 빈 diff로 이전 변경을 지우며 모든 종료에서 한 번 닫는다.
#[test]
fn aggregate_turn_diff_replaces_content_and_closes_with_the_turn() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};
    for status in ["completed", "failed", "interrupted"] {
        let session_id = session(1);
        let update = |diff: &str| json!({"method":"turn/diff/updated","params":{"threadId":"thread-a","turnId":"turn-a","diff":diff}});
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            update("diff --git a/a.rs b/a.rs\n-old\n+new\n"),
            update(""),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"failed turn"}}}}),
            update("late"),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::FileChange,
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing aggregate start")
        };
        for expected in [
            "Turn aggregate diff\ndiff --git a/a.rs b/a.rs\n-old\n+new\n",
            "Turn aggregate diff\nNo remaining changes reported for this turn.",
        ] {
            assert_eq!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(expected.to_owned())
                })
            );
        }
        let BackendPoll::Event(BackendEvent::ActivityFinished {
            activity: finished,
            outcome,
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing aggregate completion")
        };
        assert_eq!(finished, activity);
        assert!(match status {
            "completed" => outcome == ActivityOutcome::Completed,
            "interrupted" => outcome == ActivityOutcome::Interrupted,
            _ => matches!(outcome, ActivityOutcome::Failed(_)),
        });
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(
                BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
            )
        ));
        assert!(backend.turn_diffs.is_empty());
        assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    }
}

// 누적 diff의 대상·타입과 정확한 크기 한도를 검증하고 거부 시 부분 activity를 남기지 않는다.
#[test]
fn aggregate_turn_diff_validates_targets_and_exact_size_before_publication() {
    use yo_core::{BackendEvent, BackendPoll};
    let capacity = ToolOutput::MAX_SNAPSHOT_BYTES - "Turn aggregate diff\n".len();
    for (field, value, accepted) in [
        ("threadId", json!("other"), false),
        ("turnId", json!("other"), false),
        ("diff", json!(null), false),
        ("diff", json!("x".repeat(capacity)), true),
        ("diff", json!("x".repeat(capacity + 1)), false),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","diff":""});
        params[field] = value;
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"turn/diff/updated","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        if accepted {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::FileChange,
                    ..
                })
            ));
        } else {
            assert_eq!(
                backend.poll_event().unwrap_err().kind(),
                BackendFailureKind::Protocol
            );
            assert!(backend.pending_events.is_empty());
            assert!(backend.turn_diffs.is_empty());
        }
    }
}
