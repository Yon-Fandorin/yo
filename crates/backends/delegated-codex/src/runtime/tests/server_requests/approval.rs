use super::{super::support::*, *};

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
