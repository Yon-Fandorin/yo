use std::iter;

use yo_core::{ActivityApproval, ToolOutput};

use super::super::*;

// 빈 title도 name과 rawInput에서 실행 내용을 복원할 수 있으면 operation이 보이는 단발
// 승인으로 투영하고, 선택한 원래 optionId를 동일 wire request에 돌려줍니다.
#[test]
fn maps_permission_options_and_returns_the_selected_once_decision() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        json!({
            "jsonrpc": "2.0",
            "id": "permission-a",
            "method": "session/request_permission",
            "params": {
                "sessionId": "grok-session-a",
                "toolCall": {
                    "toolCallId": "tool-a",
                    "title": "",
                    "name": "terminal",
                    "rawInput": { "command": "cargo test" }
                },
                "options": [
                    { "optionId": "always", "name": "Always", "kind": "allow_always" },
                    { "optionId": "once", "name": "Once", "kind": "allow_once" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" }
                ]
            }
        }),
        response(4, json!({ "stopReason": "end_turn" })),
    ];
    let (mut backend, sent) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ApprovalRequest { request_id },
    }) = backend.poll_event().unwrap()
    else {
        panic!("permission must start an approval request");
    };
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: observed,
            update: yo_core::ActivityUpdate::TextSnapshot(summary),
        }) if observed == activity && ActivityApproval::from_snapshot(&summary).is_some_and(|profile| profile.plain_text.starts_with("terminal: {\"command\":\"cargo test\"}"))
    ));
    let request = yo_core::ActivityRequestRef::new(activity, request_id);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();

    let sent = sent.0.borrow();
    let response = sent.last().unwrap();
    assert_eq!(response["id"], "permission-a");
    assert_eq!(response["result"]["outcome"]["outcome"], "selected");
    assert_eq!(response["result"]["outcome"]["optionId"], "once");
}

// process가 제한 argv를 무시하거나 vendor 동작이 바뀌어 permission request가 와도,
// read-only review는 사용자 승인으로 승격하지 않고 wire 거절 뒤 Protocol 실패합니다.
#[test]
fn read_only_review_rejects_permission_requests_without_an_approval_activity() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend_with_profile(
        [
            response(3, json!({ "sessionId": "grok-session-a" })),
            permission_request("permission-a", Some("Run cargo test")),
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
    assert!(backend.approvals.is_empty());
    assert!(backend.pending_events.is_empty());
    let rejection = sent.0.borrow().last().cloned().unwrap();
    assert_eq!(rejection["id"], "permission-a");
    assert_eq!(rejection["error"]["code"], -32000);
}

// permission request가 실행 가능한 tool 제목 없이 도착하면 승인 Activity를 만들거나
// 사용자에게 빈 요청을 노출하지 않고 Protocol 실패로 닫습니다.
#[test]
fn rejects_permission_requests_without_an_actionable_tool_title() {
    for tool_call in [
        json!({}),
        json!({ "title": "" }),
        json!({ "title": "   " }),
        json!({ "name": "terminal", "rawInput": { "command": "   " } }),
        json!({ "name": "terminal", "rawInput": [null, " "] }),
        json!({ "name": "terminal", "rawInput": { "nested": { "value": null } } }),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let mut permission = permission_request("permission-a", None);
        permission["params"]["toolCall"] = tool_call;
        let (mut backend, sent) = backend([
            response(3, json!({ "sessionId": "grok-session-a" })),
            permission,
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("test"),
            })
            .unwrap();

        let failure = backend.poll_event().unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Protocol);
        assert!(failure.message().contains("actionable"));
        assert!(backend.approvals.is_empty());
        assert!(backend.pending_events.is_empty());
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({
                "jsonrpc": "2.0",
                "id": "permission-a",
                "result": { "outcome": { "outcome": "selected", "optionId": "reject" } }
            })
        );
    }
}

// Yo의 이진 승인은 단발 결정이므로 peer가 persistent 선택지만 제시할 때 이를 선택해
// 권한을 확대하지 않고 Protocol 실패로 닫습니다.
#[test]
fn rejects_permission_requests_without_once_options() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        json!({
            "jsonrpc": "2.0",
            "id": "permission-a",
            "method": "session/request_permission",
            "params": {
                "sessionId": "grok-session-a",
                "options": [
                    { "optionId": "always", "kind": "allow_always" },
                    { "optionId": "never", "kind": "reject_always" }
                ]
            }
        }),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("allow_once"));
}

// 승인 선택은 원래 순번·범위·wire ID를 유지하고 관련 파일은 같은 미종료 호출에만 연결한다.
#[test]
fn grok_approval_choices_preserve_scope_and_exact_file_binding() {
    use yo_core::{ActivityRequestRef, ActivityUpdate};
    for (kind, call_id, status, linked) in [
        ("edit", "tool-a", "pending", true),
        ("read", "tool-a", "pending", false),
        ("edit", "other", "pending", false),
        ("edit", "tool-a", "completed", false),
    ] {
        for (ordinal, selected) in [(1, "always"), (3, "reject"), (4, "once"), (5, "never")] {
            let session_id = session(1);
            let (mut backend, sent) = backend([
                response(3, json!({"sessionId":"grok-session-a"})),
                session_update(
                    "tool_call",
                    json!({"toolCallId":call_id,"kind":kind,"status":status,"title":"Edit settings"}),
                ),
                json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                    "sessionId":"grok-session-a","toolCall":{"toolCallId":"tool-a","title":"Edit settings","rawInput":{"path":"settings.rs"}},
                    "options":[
                        {"optionId":"always","name":"Remember allowance","kind":"allow_always"},
                        {"optionId":"unknown","name":"Future decision","kind":"future"},
                        {"optionId":"reject","name":"Reject once","kind":"reject_once"},
                        {"optionId":"once","name":"Allow once","kind":"allow_once"},
                        {"optionId":"never","name":"Remember rejection","kind":"reject_always"}
                    ]
                }}),
            ]);
            create_session(&mut backend, session_id);
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: UserInput::from("edit"),
                })
                .unwrap();
            let mut file = None;
            let mut request = None;
            let mut profile = None;
            for _ in 0..20 {
                match backend.poll_event().unwrap() {
                    BackendPoll::Event(BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::FileChange,
                    }) => file = Some(activity.activity_id().get().get()),
                    BackendPoll::Event(BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::ApprovalRequest { request_id },
                    }) => request = Some(ActivityRequestRef::new(activity, request_id)),
                    BackendPoll::Event(BackendEvent::ActivityUpdated {
                        update: ActivityUpdate::TextSnapshot(text),
                        ..
                    }) => {
                        if let Some(value) = ActivityApproval::from_snapshot(&text) {
                            profile = Some(value);
                            break;
                        }
                    },
                    _ => {},
                }
            }
            let profile = profile.unwrap();
            assert_eq!(profile.related_change, if linked { file } else { None });
            assert_eq!(profile.decline_choice, Some(3));
            assert_eq!(profile.choices.len(), 5);
            assert!(!profile.choices[1].enabled);
            assert!(profile.choices[0].description.contains("remember"));
            assert!(profile.plain_text.contains("settings.rs"));
            let request = request.unwrap();
            for invalid in [0, 2, 6] {
                assert!(
                    backend
                        .execute_command(AgentCommand::RespondToActivity {
                            request,
                            response: ActivityResponse::Approval(ApprovalDecision::Offered(
                                invalid
                            )),
                        })
                        .is_err()
                );
                assert!(backend.approvals.contains_key(&request));
                assert!(
                    !sent
                        .0
                        .borrow()
                        .iter()
                        .any(|message| message["id"] == "permission-a")
                );
            }
            backend
                .execute_command(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(ApprovalDecision::Offered(ordinal)),
                })
                .unwrap();
            assert_eq!(
                sent.0.borrow().last().unwrap()["result"]["outcome"]["optionId"],
                selected
            );
            assert!(!backend.approvals.contains_key(&request));
            let mut receipt = None;
            for _ in 0..4 {
                if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                    update: ActivityUpdate::TextSnapshot(text),
                    ..
                }) = backend.poll_event().unwrap()
                {
                    receipt = Some(text);
                }
            }
            let receipt = receipt.expect("sent decision receipt");
            assert!(receipt.contains(&profile.choices[ordinal as usize - 1].label));
            assert!(receipt.contains(&profile.choices[ordinal as usize - 1].description));
            assert!(!receipt.contains("settings.rs"));

            assert!(
                backend
                    .execute_command(AgentCommand::RespondToActivity {
                        request,
                        response: ActivityResponse::Approval(ApprovalDecision::Offered(ordinal)),
                    })
                    .is_err()
            );
        }
    }
}

// 승인 선택지 한도와 중복 ID는 게시 전에 검사하고 표시 한도 초과는 거절 응답으로 닫는다.
#[test]
fn grok_approval_bounds_and_duplicate_ids_fail_before_publication() {
    for mode in ["exact", "excess", "duplicate", "oversized"] {
        let count = if mode == "excess" { 65 } else { 64 };
        let mut options = (0..count).map(|index| json!({
            "optionId":format!("option-{index}"), "name":format!("Choice {index}"),
            "kind":if index == 0 { "allow_once" } else if index == 1 { "reject_once" } else { "allow_always" }
        })).collect::<Vec<_>>();
        if mode == "duplicate" {
            options[2]["optionId"] = options[0]["optionId"].clone();
        }
        let session_id = session(1);
        let (mut backend, sent) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                "sessionId":"grok-session-a", "toolCall":{"title":"Review edit", "rawInput":if mode == "oversized" { "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES) } else { "small".into() }}, "options":options
            }}),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        if mode == "exact" {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::ApprovalRequest { .. },
                    ..
                })
            ));
            let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: yo_core::ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
            else {
                panic!("approval profile required")
            };
            assert_eq!(
                ActivityApproval::from_snapshot(&text)
                    .unwrap()
                    .choices
                    .len(),
                64
            );
        } else {
            assert!(backend.poll_event().is_err());
            assert!(backend.approvals.is_empty());
            assert!(backend.pending_events.is_empty());
            if mode == "oversized" {
                assert_eq!(
                    sent.0.borrow().last().unwrap()["result"]["outcome"]["optionId"],
                    "option-1"
                );
            }
        }
    }
}

// 승인 전후 순서가 달라도 정확한 미종료 호출만 연결하고 ID 전용 요청은 관측된 설명을 사용한다.
#[test]
fn grok_permission_links_late_calls_and_resolves_known_details() {
    use yo_core::ActivityUpdate;
    for late in [false, true] {
        let session_id = session(1);
        let call = session_update(
            "tool_call",
            json!({"toolCallId":"target","kind":"edit","title":"Update settings","rawInput":{"path":"settings.rs"}}),
        );
        let permission = json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
            "sessionId":"grok-session-a","toolCall":if late { json!({"toolCallId":"target","title":"Review settings"}) } else { json!({"toolCallId":"target"}) },
            "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
        }});
        let unrelated = session_update(
            "tool_call",
            json!({"toolCallId":"other","kind":"edit","title":"Other file"}),
        );
        let events = if late {
            vec![permission, unrelated, call]
        } else {
            vec![call, unrelated, permission]
        };
        let (mut backend, _) =
            backend(iter::once(response(3, json!({"sessionId":"grok-session-a"}))).chain(events));
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut profiles = Vec::new();
        let mut owner = None;
        for _ in 0..30 {
            if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) = backend.poll_event().unwrap()
                && let Some(profile) = ActivityApproval::from_snapshot(&text)
            {
                if let Some(owner) = owner {
                    assert_eq!(activity, owner);
                } else {
                    owner = Some(activity);
                }
                profiles.push(profile);
            }
            if profiles.len() == if late { 2 } else { 1 } {
                break;
            }
        }
        let expected = backend.tools["target"].activity.activity_id().get().get();
        let profile = profiles.last().unwrap();
        assert_eq!(profile.related_change, Some(expected));
        assert_ne!(
            profile.related_change,
            Some(backend.tools["other"].activity.activity_id().get().get())
        );
        if late {
            assert_eq!(profiles[0].related_change, None);
            assert_eq!(profiles[0].choices, profiles[1].choices);
            assert_eq!(profiles[0].plain_text, profiles[1].plain_text);
        } else {
            assert!(profile.plain_text.contains("Update settings"));
            assert!(profile.plain_text.contains("settings.rs"));
        }
    }
}

// 승인 요청의 명시적 새 인수는 이전 호출 인수보다 우선하며 예약 링크 바이트도 한도에 포함한다.
#[test]
fn grok_permission_summary_uses_current_arguments_and_reserves_link_bytes() {
    use yo_core::ActivityUpdate;
    let mut reserve = 0;
    for mode in ["arguments", "measure", "exact", "excess"] {
        let session_id = session(1);
        let mut events = vec![response(3, json!({"sessionId":"grok-session-a"}))];
        if mode == "arguments" {
            events.push(session_update("tool_call",json!({"toolCallId":"target","name":"terminal","rawInput":{"command":"old-command"}})));
        }
        let title_len = match mode {
            "exact" => 1 + reserve,
            "excess" => 2 + reserve,
            _ => 1,
        };
        let tool_call = if mode == "arguments" {
            json!({"toolCallId":"target","rawInput":{"command":"new-command"}})
        } else {
            json!({"toolCallId":"target","title":"x".repeat(title_len)})
        };
        events.push(json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
            "sessionId":"grok-session-a","toolCall":tool_call,
            "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
        }}));
        let (mut backend, sent) = backend(events);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut observed = None;
        let mut rejected = false;
        for _ in 0..12 {
            match backend.poll_event() {
                Ok(BackendPoll::Event(BackendEvent::ActivityUpdated {
                    update: ActivityUpdate::TextSnapshot(text),
                    ..
                })) => {
                    if let Some(profile) = ActivityApproval::from_snapshot(&text) {
                        observed = Some(profile);
                        break;
                    }
                },
                Err(_) => {
                    rejected = true;
                    break;
                },
                _ => {},
            }
        }
        if mode == "excess" {
            assert!(rejected);
            assert!(backend.approvals.is_empty());
            assert_eq!(
                sent.0.borrow().last().unwrap()["result"]["outcome"]["optionId"],
                "no"
            );
        } else {
            let mut profile = observed.expect("admitted approval");
            if mode == "arguments" {
                assert!(
                    profile
                        .plain_text
                        .contains("terminal: {\"command\":\"new-command\"}")
                );
                assert!(!profile.plain_text.contains("old-command"));
            } else {
                profile.related_change = Some(u64::MAX);
                let length = profile.to_snapshot().unwrap().len();
                if mode == "measure" {
                    reserve = ToolOutput::MAX_SNAPSHOT_BYTES - length;
                } else {
                    assert_eq!(length, ToolOutput::MAX_SNAPSHOT_BYTES);
                }
            }
        }
    }
}

// 승인 요청의 최신 diff는 승인 게시 전에 반영하지만 요청의 상태·종류로 호출을 끝내거나 바꾸지
// 않는다.
#[test]
fn grok_permission_refreshes_diff_before_approval_without_finishing_call() {
    use yo_core::ActivityUpdate;
    for clear in [false, true] {
        let session_id = session(1);
        let old =
            json!({"type":"diff","path":"settings.rs","oldText":"base\n","newText":"stale\n"});
        let new =
            json!({"type":"diff","path":"settings.rs","oldText":"base\n","newText":"reviewed\n"});
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update(
                "tool_call",
                json!({"toolCallId":"target","kind":"edit","title":"Edit settings","content":[old]}),
            ),
            json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                "sessionId":"grok-session-a","toolCall":{"toolCallId":"target","title":"Review settings","status":"completed","kind":"read","rawInput":{"path":"current.rs"},"content":if clear {json!([])} else {json!([new.clone()])}},
                "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
            }}),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut call = None;
        let mut latest = String::new();
        let mut output = None;
        let mut approved = false;
        for _ in 0..30 {
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
                }) => {
                    if Some(activity) == call {
                        latest = text;
                    } else if let Some(value) = ToolOutput::from_snapshot(&text) {
                        output = Some(value);
                    }
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
                    if Some(activity) == call =>
                {
                    panic!("permission status must not complete call")
                },
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::ApprovalRequest { .. },
                    ..
                }) => {
                    approved = true;
                    break;
                },
                _ => {},
            }
        }
        assert!(approved);
        assert!(!latest.contains("+stale"));
        assert_eq!(latest.contains("+reviewed"), !clear);
        let output = output.unwrap();
        assert_eq!(output.arguments.unwrap()["path"], "current.rs");
        if clear {
            assert_eq!(output.content_items, Some(json!([])));
        } else {
            assert_eq!(output.content_items.unwrap()[0]["source"], new);
        }
        assert!(!backend.tools["target"].finished);
        assert!(backend.tools["target"].file_change);
    }
}

// 선행 승인 diff는 원문에 남고 실제 호출의 생략 필드만 보충하며 새 값·명시적 빈 값은 덮어쓰지
// 않는다.
#[test]
fn grok_early_permission_retains_diff_without_overwriting_call_content() {
    use yo_core::ActivityUpdate;
    for mode in ["omitted", "newer", "empty"] {
        let session_id = session(1);
        let proposed =
            json!({"type":"diff","path":"a.rs","oldText":"base\n","newText":"proposed\n"});
        let newer = json!({"type":"diff","path":"a.rs","oldText":"base\n","newText":"actual\n"});
        let mut call = json!({"toolCallId":"target","kind":"edit","title":"Edit file","rawInput":{"path":"actual.rs"}});
        if mode == "newer" {
            call["content"] = json!([newer.clone()]);
        }
        if mode == "empty" {
            call["content"] = json!([]);
        }
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                "sessionId":"grok-session-a","toolCall":{"toolCallId":"target","title":"Review file","rawInput":{"path":"proposed.rs"},"content":[proposed.clone()],"locations":[{"path":"a.rs","line":1}]},
                "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
            }}),
            session_update(
                "tool_call",
                json!({"toolCallId":"other","kind":"edit","title":"Other file"}),
            ),
            session_update("tool_call", call),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut profile: Option<String> = None;
        let mut output = None;
        let mut linked = false;
        for _ in 0..30 {
            if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
            {
                if let Some(value) = ActivityApproval::from_snapshot(&text) {
                    if value.related_change.is_some() {
                        assert_eq!(&value.plain_text, profile.as_ref().unwrap());
                        linked = true;
                        break;
                    }
                    assert!(
                        value
                            .plain_text
                            .contains(&format!("{:#}", json!([proposed.clone()])))
                    );
                    profile = Some(value.plain_text);
                } else if let Some(value) = ToolOutput::from_snapshot(&text) {
                    output = Some(value);
                }
            }
        }
        assert!(linked);
        let output = output.unwrap();
        assert_eq!(output.arguments.unwrap()["path"], "actual.rs");
        assert_eq!(output.result.unwrap()["locations"][0]["path"], "a.rs");
        if mode == "empty" {
            assert_eq!(output.content_items, Some(json!([])));
        } else {
            assert_eq!(
                output.content_items.unwrap()[0]["source"],
                if mode == "newer" { newer } else { proposed }
            );
        }
        assert!(
            backend
                .approvals
                .values()
                .all(|binding| binding.pending_display.is_none())
        );
        assert_eq!(backend.tools["other"].output, json!({}));
    }
}
