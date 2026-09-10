use std::time::Duration;

use yo_core::{ActivityKind, ActivityRequestRef, AgentEvent, ApprovalDecision, RequestId};

use super::{activity, key, nonzero};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

// outstanding approval이 있을 때 `y` 제출은 새 Turn이나 steer가 아니라 원래 Activity와
// request ID를 가진 승인 응답 action이 된다.
#[test]
fn converts_yes_into_a_correlated_approval_response() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(7));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("y".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(request_activity, request_id),
            decision: ApprovalDecision::Approved,
        })
    );
}

// agent가 추가 입력을 요청한 동안 제출한 문자열은 활성 Turn steer가 아니라 원래
// Activity와 request ID를 가진 UserInput 응답 action으로 변환된다.
#[test]
fn converts_text_into_a_correlated_agent_input_response() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(8));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(
            InputEvent::Paste("use the second option".to_owned()),
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "use the second option".to_owned(),
        })
    );
}

// 서로 다른 Activity의 approval·user-input request가 동시에 대기하면 첫 Enter는 queue 앞의
// request에만 응답하고, 다음 Enter는 뒤의 request와 그 request ID를 그대로 상관시킨다.
#[test]
fn multiple_pending_requests_are_answered_fifo_with_their_own_correlations() {
    let mut state = TuiState::new();
    let approval_activity = activity(1);
    let approval_id = RequestId::new(nonzero(7));
    let input_activity = activity(2);
    let input_id = RequestId::new(nonzero(8));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: approval_activity,
            kind: ActivityKind::ApprovalRequest {
                request_id: approval_id,
            },
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityStarted {
            activity: input_activity,
            kind: ActivityKind::UserInputRequest {
                request_id: input_id,
            },
        })
        .unwrap();

    state
        .handle(InputEvent::Paste("y".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(approval_activity, approval_id),
            decision: ApprovalDecision::Approved,
        })
    );
    assert!(state.has_pending_request());

    state
        .handle(
            InputEvent::Paste("use the second option".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(input_activity, input_id),
            input: "use the second option".to_owned(),
        })
    );
    assert!(!state.has_pending_request());
}

// pending Activity가 있어도 /help는 로컬에서 실행되고 request를 답하거나 취소하지 않는다.
#[test]
fn local_help_does_not_answer_an_outstanding_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(9));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/help".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.has_pending_request());
    assert!(state.editor().text().is_empty());
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Available commands"), "{output}");
}

// cursor 뒤 text 때문에 palette가 소유하지 않는 slash draft는 known invocation이어도
// local command로 재분류되지 않고 대기 중인 Activity의 correlated response가 된다.
#[test]
fn cursor_ineligible_command_draft_answers_the_outstanding_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(13));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/help".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();

    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "/help".to_owned(),
        })
    );
}

// pending Activity 중 unknown slash draft도 표시된 palette를 Esc로 닫은 경우에만 원래
// Activity의 correlated input response로 전달된다.
#[test]
fn escaped_command_draft_answers_the_outstanding_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(10));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();
    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "/foo".to_owned(),
        })
    );
}

// /exit는 process lifecycle 명령이므로 pending Activity를 답으로 소비하지 않고 즉시
// 기존 runner 종료 경계를 사용한다.
#[test]
fn exit_remains_an_explicit_process_lifecycle_exception_during_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(12));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/exit".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert!(state.has_pending_request());
}

fn present_request(state: &mut TuiState) {
    let frame = state
        .prepare_frame(Size::new(88, 30), &AppearanceState::default().pin())
        .unwrap();
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
}

// 파일을 선택한 초안은 이전 질문 이동과 메모 제출에서도 문자열로 축소하지 않고 보존한다.
#[test]
fn previous_question_preserves_selected_reference_draft() {
    use yo_core::{
        ActivityQuestion, ActivityUpdate, QuestionChoice, WorkspaceReference,
        WorkspaceReferenceCandidate, WorkspaceReferenceKind, WorkspaceReferenceSearchStatus,
        WorkspaceReferenceSearchUpdate,
    };

    let mut state = TuiState::new();
    state.enable_workspace_references();
    let StateEffect::WorkspaceSearch(search) = state
        .handle(InputEvent::Paste("@src".into()), Duration::ZERO)
        .unwrap()
    else {
        panic!("workspace search expected")
    };
    let reference = WorkspaceReference::new(
        "file:one",
        "host:one",
        "workspace:one",
        "root:one",
        "src/main.rs",
        WorkspaceReferenceKind::File,
    )
    .unwrap();
    state.observe_workspace_reference_update(WorkspaceReferenceSearchUpdate::final_result(
        &search,
        WorkspaceReferenceSearchStatus::Complete,
        vec![WorkspaceReferenceCandidate::new(reference)],
    ));
    present_request(&mut state);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let draft = state.editor().text().to_owned();
    let request_id = RequestId::new(nonzero(97));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                ActivityQuestion {
                    plain_text: "Second question".into(),
                    choices: vec![QuestionChoice {
                        label: "UI".into(),
                        description: "Layout".into(),
                    }],
                    allow_notes: true,
                    previous_question: true,
                    draft: None,
                    draft_choice: None,
                }
                .to_snapshot()
                .unwrap(),
            ),
        })
        .unwrap();
    present_request(&mut state);
    assert!(!matches!(
        state
            .handle(key(KeyCode::BackTab, KeyModifiers::SHIFT), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(_)
    ));
    assert_eq!(state.editor().text(), draft);
    assert!(state.has_pending_request());
    present_request(&mut state);
    assert_eq!(
        state
            .handle(key(KeyCode::Tab, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    present_request(&mut state);
    assert!(!matches!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(_)
    ));
    assert_eq!(state.editor().text(), draft);
    assert!(state.has_pending_request());

    present_request(&mut state);
    state
        .handle(
            key(KeyCode::Character('u'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    state
        .handle(InputEvent::Paste("plain notes".into()), Duration::ZERO)
        .unwrap();
    present_request(&mut state);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToQuestion {
            request: ActivityRequestRef::new(activity(1), request_id),
            choice: 1,
            notes: "plain notes".into(),
        })
    );
}

// 승인 panel은 표시 전 Enter를 무시하고, 기본 거절과 명시적으로 이동한 한 번 승인을
// 각각 원래 요청 ID로 전달해야 한다. Esc도 동일한 거절 응답이다.
#[test]
fn approval_panel_requires_presentation_and_explicit_selection() {
    for (select_allow, escape) in [(false, false), (true, false), (true, true)] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(17));
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ApprovalRequest { request_id },
            })
            .unwrap();
        assert_eq!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Unchanged
        );
        present_request(&mut state);
        if select_allow {
            state
                .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
                .unwrap();
            present_request(&mut state);
        }
        let code = if escape {
            KeyCode::Escape
        } else {
            KeyCode::Enter
        };
        assert_eq!(
            state
                .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(AgentAction::RespondToApproval {
                request: ActivityRequestRef::new(activity(1), request_id),
                decision: if select_allow && !escape {
                    ApprovalDecision::Approved
                } else {
                    ApprovalDecision::Declined
                },
            })
        );
        assert!(!state.has_pending_request());
    }
}

// 질문 panel이 표시된 상태에서도 자유 입력은 질문 응답으로 전달되고, Esc는 중단으로
// 전달되어 빈 답변이나 새 Turn을 생성하지 않는다.
#[test]
fn visible_interview_accepts_text_or_cancels() {
    for cancel in [false, true] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(18));
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::UserInputRequest { request_id },
            })
            .unwrap();
        present_request(&mut state);
        state
            .handle(InputEvent::Paste("직접 쓴 답변".to_owned()), Duration::ZERO)
            .unwrap();
        present_request(&mut state);
        assert_eq!(
            state
                .handle(
                    key(
                        if cancel {
                            KeyCode::Escape
                        } else {
                            KeyCode::Enter
                        },
                        KeyModifiers::NONE
                    ),
                    Duration::ZERO
                )
                .unwrap(),
            StateEffect::Dispatch(if cancel {
                AgentAction::Interrupt
            } else {
                AgentAction::RespondToUserInput {
                    request: ActivityRequestRef::new(activity(1), request_id),
                    input: "직접 쓴 답변".to_owned(),
                }
            })
        );
    }
}

// 승인 내용이 갱신되면 이미 선택한 허용도 새 frame이 표시되기 전까지 실행하지 않는다.
#[test]
fn changed_approval_content_requires_a_new_presented_frame() {
    use yo_core::ActivityUpdate;

    let mut state = TuiState::new();
    let request_id = RequestId::new(nonzero(19));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    present_request(&mut state);
    state
        .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    present_request(&mut state);
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("Updated command".into()),
        })
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.has_pending_request());
    present_request(&mut state);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(activity(1), request_id),
            decision: ApprovalDecision::Approved,
        })
    );
}

// 출력 탐색 중 도착한 질문은 읽기 키로 답하지 않고 Chat 복귀 뒤 원래 request에 응답한다.
#[test]
fn output_navigation_preserves_an_outstanding_question() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/output".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(91));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    for event in [
        InputEvent::Paste("not an answer".into()),
        key(KeyCode::Enter, KeyModifiers::NONE),
        key(KeyCode::Down, KeyModifiers::NONE),
    ] {
        assert!(!matches!(
            state.handle(event, Duration::ZERO).unwrap(),
            StateEffect::Dispatch(_)
        ));
    }
    assert!(state.has_pending_request());
    assert!(state.editor().text().is_empty());
    state
        .handle(
            key(KeyCode::Function(1), KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap();
    state
        .handle(InputEvent::Paste("actual answer".into()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "actual answer".into(),
        })
    );
}

// 구조화 선택지는 실제 표시 뒤에만 선택하며 두 번째 항목도 원래 요청 ID와 숫자 응답으로 연결한다.
// 일반 원문 내보내기에는 질문을 남기고 JSON 스키마를 노출하지 않는다.
#[test]
fn structured_question_choices_require_presentation_and_preserve_request_identity() {
    use yo_core::{ActivityQuestion, ActivityUpdate, QuestionChoice};
    let mut state = TuiState::new();
    let request_id = RequestId::new(nonzero(8));
    let request = ActivityRequestRef::new(activity(1), request_id);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    let profile = ActivityQuestion {
        allow_notes: true,
        previous_question: false,
        draft: None,
        draft_choice: None,
        plain_text: "Which area?".into(),
        choices: vec![
            QuestionChoice {
                label: "UI".into(),
                description: "Layout".into(),
            },
            QuestionChoice {
                label: "Runtime".into(),
                description: "Events".into(),
            },
        ],
    };
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
        })
        .unwrap();
    assert!(!matches!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(_)
    ));
    let pin = AppearanceState::default().pin();
    let frame = state.prepare_frame(Size::new(80, 20), &pin).unwrap();
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
    let mut replacement = profile.clone();
    replacement.choices[0].description = "Updated layout details".into();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(replacement.to_snapshot().unwrap()),
        })
        .unwrap();
    assert!(!matches!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(_)
    ));
    let frame = state.prepare_frame(Size::new(80, 20), &pin).unwrap();
    state.commit_frame(&frame);
    state
        .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let frame = state.prepare_frame(Size::new(24, 20), &pin).unwrap();
    state.commit_frame(&frame);
    let output = state.session_output(&pin).unwrap().unwrap();
    assert!(output.contains("Which area?"));
    assert!(!output.contains(ActivityQuestion::SCHEMA));
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request,
            input: "2".into()
        })
    );
}

// Tab은 표시된 선택지를 메모와 묶되 새 panel 표시 전 제출을 차단한다. 좁은 폭과
// slash/다중 행 입력에서도 메모는 로컬 명령이나 새 Turn이 아닌 같은 질문의 답변이다.
#[test]
fn question_notes_keep_selection_and_literal_text_after_narrow_reflow() {
    use yo_core::{ActivityQuestion, ActivityUpdate, QuestionChoice};
    for (allow_notes, notes) in [(false, ""), (true, ""), (true, "/exit\n추가 설명 2")] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(81));
        let request = ActivityRequestRef::new(activity(1), request_id);
        let profile = ActivityQuestion {
            allow_notes,
            previous_question: false,
            draft: None,
            draft_choice: None,
            plain_text: "Choose an area".into(),
            choices: vec![
                QuestionChoice {
                    label: "UI".into(),
                    description: "Layout".into(),
                },
                QuestionChoice {
                    label: "Runtime".into(),
                    description: "Events".into(),
                },
            ],
        };
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::UserInputRequest { request_id },
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        present_request(&mut state);
        state
            .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        present_request(&mut state);
        let tab = state
            .handle(key(KeyCode::Tab, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        if !allow_notes {
            assert_eq!(
                tab,
                StateEffect::Dispatch(AgentAction::RespondToUserInput {
                    request,
                    input: "2".into()
                })
            );
            continue;
        }
        assert_eq!(tab, StateEffect::Redraw);
        state
            .handle(InputEvent::Paste(notes.into()), Duration::ZERO)
            .unwrap();
        assert!(!matches!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        assert_eq!(state.editor().text(), notes);
        let pin = AppearanceState::default().pin();
        let frame = state.prepare_frame(Size::new(24, 24), &pin).unwrap();
        assert!(frame.overlay_presented);
        state.commit_frame(&frame);
        assert_eq!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(AgentAction::RespondToQuestion {
                request,
                choice: 2,
                notes: notes.into()
            })
        );
        assert!(state.editor().text().is_empty());
        assert!(!state.has_pending_request());
    }
}

// 질문 변경은 메모에 묶인 이전 선택을 무효화한다. Tab으로 목록에 돌아와도 메모 원문은
// 보존하며 새 선택을 표시한 뒤에만 다시 묶어 보낼 수 있다.
#[test]
fn question_notes_refresh_and_return_to_choices_preserve_draft_without_stale_selection() {
    use yo_core::{ActivityQuestion, ActivityUpdate, QuestionChoice};
    for refresh in [false, true] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(82));
        let request = ActivityRequestRef::new(activity(1), request_id);
        let mut profile = ActivityQuestion {
            allow_notes: true,
            previous_question: false,
            draft: None,
            draft_choice: None,
            plain_text: "Choose".into(),
            choices: vec![QuestionChoice {
                label: "First".into(),
                description: "Original".into(),
            }],
        };
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::UserInputRequest { request_id },
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        present_request(&mut state);
        state
            .handle(key(KeyCode::Tab, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        present_request(&mut state);
        state
            .handle(InputEvent::Paste("memo".into()), Duration::ZERO)
            .unwrap();
        if refresh {
            profile.choices[0].label = "Replacement".into();
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(1),
                    update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
                })
                .unwrap();
        } else {
            state
                .handle(key(KeyCode::Tab, KeyModifiers::NONE), Duration::ZERO)
                .unwrap();
        }
        if refresh {
            assert!(!matches!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Dispatch(_)
            ));
        }
        assert_eq!(state.editor().text(), "memo");
        assert!(!matches!(
            state
                .handle(key(KeyCode::Tab, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        present_request(&mut state);
        assert_eq!(
            state
                .handle(key(KeyCode::Tab, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Redraw
        );
        present_request(&mut state);
        assert_eq!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(AgentAction::RespondToQuestion {
                request,
                choice: 1,
                notes: "memo".into()
            })
        );
    }
}

// 표시 전과 갱신 직후에는 승인할 수 없고 거절 우선 정렬 뒤에도 원래 선택지 번호를 전송한다.
// 좁은 폭·이력 내보내기는 실제 문맥을 보존하며 프로필 JSON을 본문에 노출하지 않는다.
#[test]
fn offered_approval_choices_require_visible_frame_and_preserve_original_ordinal() {
    use yo_core::{ActivityApproval, ActivityUpdate, ApprovalChoice};
    for escape in [false, true] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(8));
        let request = ActivityRequestRef::new(activity(1), request_id);
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ApprovalRequest { request_id },
            })
            .unwrap();
        let mut profile = ActivityApproval {
            related_change: None,
            plain_text: "Command: cargo test\nWorking directory: /workspace/yo".into(),
            choices: vec![
                ApprovalChoice {
                    label: "Approve for session".into(),
                    description: "Future matching commands in this session".into(),
                    enabled: true,
                },
                ApprovalChoice {
                    label: "Decline".into(),
                    description: "Continue without running".into(),
                    enabled: true,
                },
            ],
            decline_choice: Some(2),
        };
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        assert!(!matches!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        let pin = AppearanceState::default().pin();
        let frame = state.prepare_frame(Size::new(80, 30), &pin).unwrap();
        state.commit_frame(&frame);
        profile.related_change = Some(9);
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        assert!(!matches!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        let frame = state.prepare_frame(Size::new(80, 30), &pin).unwrap();
        state.commit_frame(&frame);
        profile
            .plain_text
            .push_str("\nUpdated permission context; choices unchanged.");
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        assert!(!matches!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        state
            .handle(
                key(KeyCode::Character('1'), KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap();
        assert!(!matches!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        assert_eq!(state.editor().text(), "1");
        state
            .handle(key(KeyCode::Backspace, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        let frame = state.prepare_frame(Size::new(24, 30), &pin).unwrap();
        state.commit_frame(&frame);
        if !escape {
            state
                .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
                .unwrap();
            let frame = state.prepare_frame(Size::new(80, 30), &pin).unwrap();
            state.commit_frame(&frame);
        }
        let plain = state.session_output(&pin).unwrap().unwrap();
        assert!(plain.contains("Working directory: /workspace/yo"));
        assert!(!plain.contains(ActivityApproval::SCHEMA));
        assert_eq!(
            state
                .handle(
                    key(
                        if escape {
                            KeyCode::Escape
                        } else {
                            KeyCode::Enter
                        },
                        KeyModifiers::NONE
                    ),
                    Duration::ZERO
                )
                .unwrap(),
            StateEffect::Dispatch(AgentAction::RespondToApproval {
                request,
                decision: ApprovalDecision::Offered(if escape { 2 } else { 1 })
            })
        );
    }
}

// 거절 없는 요청의 기본 Enter/Esc는 권한을 주지 않고 중단하며 입력한 번호도 표시 전에는 제출하지
// 않는다.
#[test]
fn approval_without_decline_defaults_to_interrupt_and_blocks_unseen_typed_choices() {
    use yo_core::{ActivityApproval, ActivityUpdate, ApprovalChoice};
    for code in [KeyCode::Enter, KeyCode::Escape] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(8));
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ApprovalRequest { request_id },
            })
            .unwrap();
        let profile = ActivityApproval {
            related_change: None,
            plain_text: "Session scope".into(),
            choices: vec![ApprovalChoice {
                label: "Approve for session".into(),
                description: "Future matching commands".into(),
                enabled: true,
            }],
            decline_choice: None,
        };
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        state
            .handle(
                key(KeyCode::Character('1'), KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap();
        assert!(!matches!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        assert_eq!(state.editor().text(), "1");
        state
            .handle(key(KeyCode::Backspace, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        let pin = AppearanceState::default().pin();
        let frame = state.prepare_frame(Size::new(24, 30), &pin).unwrap();
        state.commit_frame(&frame);
        assert_eq!(
            state
                .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(AgentAction::Interrupt)
        );
    }
}

// 표시할 수 없는 선택지 문자는 중단 패널로 대체하며 번호 입력으로 숨은 권한을 승인할 수 없다.
#[test]
fn unrenderable_approval_choices_cannot_be_submitted_by_typing_their_ordinals() {
    use yo_core::{ActivityApproval, ActivityUpdate, ApprovalChoice};
    let mut state = TuiState::new();
    let request_id = RequestId::new(nonzero(8));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    let profile = ActivityApproval {
        related_change: None,
        plain_text: "Review session scope".into(),
        choices: vec![
            ApprovalChoice {
                label: "Approve\nfor session".into(),
                description: "Session scope".into(),
                enabled: true,
            },
            ApprovalChoice {
                label: "Decline".into(),
                description: "Do not run".into(),
                enabled: true,
            },
        ],
        decline_choice: Some(2),
    };
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
        })
        .unwrap();
    let pin = AppearanceState::default().pin();
    let frame = state.prepare_frame(Size::new(24, 30), &pin).unwrap();
    state.commit_frame(&frame);
    state
        .handle(
            key(KeyCode::Character('1'), KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap();
    assert!(!matches!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(_)
    ));
    assert_eq!(state.editor().text(), "1");
    let frame = state.prepare_frame(Size::new(24, 30), &pin).unwrap();
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// 승인에 연결된 첫 번째 변경은 더 최근 변경이 있어도 정확히 열리고 화면 왕복은 승인하지 않는다.
#[test]
fn approval_changes_command_opens_the_related_file_without_approving() {
    use yo_core::{ActivityApproval, ActivityOutcome, ActivityUpdate, ApprovalChoice};

    use crate::surface::{CellContent, Point, Surface};
    let visible_rows = |surface: &Surface| {
        (0..surface.size().height)
            .map(|y| {
                (0..surface.size().width)
                    .map(
                        |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                            CellContent::Grapheme { text, .. } => text.to_string(),
                            _ => " ".to_owned(),
                        },
                    )
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    for completed in [false, true] {
        for related in [1, 999] {
            let mut state = TuiState::new();
            for (id, path) in [(1, "target.rs"), (2, "unrelated.rs")] {
                state
                    .observe(AgentEvent::ActivityStarted {
                        activity: activity(id),
                        kind: ActivityKind::FileChange,
                    })
                    .unwrap();
                state
                    .observe(AgentEvent::ActivityUpdated {
                        activity: activity(id),
                        update: ActivityUpdate::TextSnapshot(format!(
                            "update: {path}\n@@ -1 +1 @@\n-old\n+new"
                        )),
                    })
                    .unwrap();
            }
            if completed {
                for id in [1, 2] {
                    state
                        .observe(AgentEvent::ActivityFinished {
                            activity: activity(id),
                            outcome: ActivityOutcome::Completed,
                        })
                        .unwrap();
                }
            }
            let request_id = RequestId::new(nonzero(8));
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: activity(3),
                    kind: ActivityKind::ApprovalRequest { request_id },
                })
                .unwrap();
            let profile = ActivityApproval {
                related_change: Some(related),
                plain_text: "Review the proposed change".into(),
                choices: vec![ApprovalChoice {
                    label: "Decline".into(),
                    description: "No grant".into(),
                    enabled: true,
                }],
                decline_choice: Some(1),
            };
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(3),
                    update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
                })
                .unwrap();
            let pin = AppearanceState::default().pin();
            let frame = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
            state.commit_frame(&frame);
            assert!(visible_rows(&frame.surface).contains("/changes: proposed files"));
            for character in "/changes".chars() {
                state
                    .handle(
                        key(KeyCode::Character(character), KeyModifiers::NONE),
                        Duration::ZERO,
                    )
                    .unwrap();
            }
            let frame = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
            state.commit_frame(&frame);
            assert!(!matches!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Dispatch(_)
            ));
            if related == 999 {
                let frame = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
                assert!(visible_rows(&frame.surface).contains("not available"));
                assert!(state.has_pending_request());
                assert!(visible_rows(&frame.surface).contains("/changes: proposed files"));
                state.commit_frame(&frame);
                assert_eq!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Dispatch(AgentAction::RespondToApproval {
                        request: ActivityRequestRef::new(activity(3), request_id),
                        decision: ApprovalDecision::Offered(1),
                    })
                );
                continue;
            }
            for width in [80, 24, 80] {
                let frame = state.prepare_frame(Size::new(width, 40), &pin).unwrap();
                let text = visible_rows(&frame.surface);
                assert!(text.contains("target.rs"), "{text}");
                assert!(!text.contains("unrelated.rs"), "{text}");
                state.commit_frame(&frame);
            }
            assert!(!matches!(
                state
                    .handle(
                        key(KeyCode::Function(1), KeyModifiers::NONE),
                        Duration::ZERO
                    )
                    .unwrap(),
                StateEffect::Dispatch(_)
            ));
            assert!(state.has_pending_request());
        }
    }
}

// 연결된 diff 변경은 이전 승인 프레임·숫자 입력을 무효화하지만 무관한 파일 변경은 승인을 막지
// 않는다.
#[test]
fn related_diff_updates_require_a_fresh_approval_frame() {
    use yo_core::{
        ActivityApproval, ActivityOutcome, ActivityRef, ActivityUpdate, ApprovalChoice, TurnId,
        TurnRef,
    };
    for (changed, completed) in [
        (1, false),
        (2, false),
        (3, false),
        (1, true),
        (2, true),
        (3, true),
    ] {
        for typed in [false, true] {
            let mut state = TuiState::new();
            let foreign = ActivityRef::new(
                TurnRef::new(activity(1).session_id(), TurnId::new(nonzero(2))),
                activity(1).activity_id(),
            );
            for (id, target) in [(1, activity(1)), (2, activity(2)), (3, foreign)] {
                state
                    .observe(AgentEvent::ActivityStarted {
                        activity: target,
                        kind: ActivityKind::FileChange,
                    })
                    .unwrap();
                state
                    .observe(AgentEvent::ActivityUpdated {
                        activity: target,
                        update: ActivityUpdate::TextSnapshot(format!(
                            "update: file-{id}.rs\n@@ -1 +1 @@\n-old\n+initial"
                        )),
                    })
                    .unwrap();
            }
            let request_id = RequestId::new(nonzero(8));
            let request = ActivityRequestRef::new(activity(3), request_id);
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: request.activity(),
                    kind: ActivityKind::ApprovalRequest { request_id },
                })
                .unwrap();
            let profile = ActivityApproval {
                related_change: Some(1),
                plain_text: "Review the proposed diff".into(),
                choices: vec![
                    ApprovalChoice {
                        label: "Approve".into(),
                        description: "Apply proposal".into(),
                        enabled: true,
                    },
                    ApprovalChoice {
                        label: "Decline".into(),
                        description: "No grant".into(),
                        enabled: true,
                    },
                ],
                decline_choice: Some(2),
            };
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: request.activity(),
                    update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
                })
                .unwrap();
            let pin = AppearanceState::default().pin();
            let frame = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
            state.commit_frame(&frame);
            state
                .handle(
                    key(
                        if typed {
                            KeyCode::Character('1')
                        } else {
                            KeyCode::Down
                        },
                        KeyModifiers::NONE,
                    ),
                    Duration::ZERO,
                )
                .unwrap();
            let stale = state.prepare_frame(Size::new(80, 40), &pin).unwrap();
            state.commit_frame(&stale);
            let target = if changed == 3 {
                foreign
            } else {
                activity(changed)
            };
            let event = if completed {
                AgentEvent::ActivityFinished {
                    activity: target,
                    outcome: ActivityOutcome::Completed,
                }
            } else {
                AgentEvent::ActivityUpdated {
                    activity: target,
                    update: ActivityUpdate::TextSnapshot(format!(
                        "update: file-{changed}.rs\n@@ -1 +1 @@\n-old\n+revised"
                    )),
                }
            };
            state.observe(event).unwrap();
            if changed == 1 {
                assert!(!matches!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Dispatch(_)
                ));
                state.commit_frame(&stale);
                assert!(!matches!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Dispatch(_)
                ));
                assert!(state.has_pending_request());
                let fresh = state.prepare_frame(Size::new(24, 40), &pin).unwrap();
                state.commit_frame(&fresh);
            }
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Dispatch(AgentAction::RespondToApproval {
                    request,
                    decision: ApprovalDecision::Offered(1)
                })
            );
        }
    }
}

// 기록 화면 왕복은 같은 요청의 선택을 유지하지만 변경된 요청이나 옛 frame으로 제출하지 않는다.
#[test]
fn request_history_roundtrip_restores_only_unchanged_selection() {
    use yo_core::{ActivityQuestion, ActivityUpdate, QuestionChoice};
    for question in [false, true] {
        for changed in [false, true] {
            let mut state = TuiState::new();
            let request_id = RequestId::new(nonzero(72));
            let request = ActivityRequestRef::new(activity(1), request_id);
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: activity(1),
                    kind: if question {
                        ActivityKind::UserInputRequest { request_id }
                    } else {
                        ActivityKind::ApprovalRequest { request_id }
                    },
                })
                .unwrap();
            let profile = ActivityQuestion {
                allow_notes: false,
                previous_question: false,
                draft: None,
                draft_choice: None,
                plain_text: "Choose an area".into(),
                choices: vec![
                    QuestionChoice {
                        label: "UI".into(),
                        description: "Layout".into(),
                    },
                    QuestionChoice {
                        label: "Runtime".into(),
                        description: "Events".into(),
                    },
                ],
            };
            if question {
                state
                    .observe(AgentEvent::ActivityUpdated {
                        activity: activity(1),
                        update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
                    })
                    .unwrap();
            }
            present_request(&mut state);
            state
                .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
                .unwrap();
            let old = state
                .prepare_frame(Size::new(88, 30), &AppearanceState::default().pin())
                .unwrap();
            state.commit_frame(&old);
            state
                .handle(
                    key(KeyCode::Function(2), KeyModifiers::NONE),
                    Duration::ZERO,
                )
                .unwrap();
            if changed {
                let snapshot = if question {
                    let mut revised = profile.clone();
                    revised.plain_text = "Revised question".into();
                    revised.to_snapshot().unwrap()
                } else {
                    "Revised command".into()
                };
                state
                    .observe(AgentEvent::ActivityUpdated {
                        activity: activity(1),
                        update: ActivityUpdate::TextSnapshot(snapshot),
                    })
                    .unwrap();
            }
            state
                .handle(
                    key(KeyCode::Function(1), KeyModifiers::NONE),
                    Duration::ZERO,
                )
                .unwrap();
            state.commit_frame(&old);
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Unchanged
            );
            present_request(&mut state);
            let action = if question {
                AgentAction::RespondToUserInput {
                    request,
                    input: if changed { "1" } else { "2" }.into(),
                }
            } else {
                AgentAction::RespondToApproval {
                    request,
                    decision: if changed {
                        ApprovalDecision::Declined
                    } else {
                        ApprovalDecision::Approved
                    },
                }
            };
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Dispatch(action)
            );
        }
    }
}

// 이전 질문 이동은 지원 프로필과 최신 표시가 있어야 하며 선택·다중 행 초안을 같은 요청에 묶는다.
// 복원한 초안은 좁은 폭에서도 유지하고 후속 스냅샷이 사용자의 수정 내용을 덮어쓰지 않는다.
#[test]
fn previous_question_requires_fresh_profile_and_preserves_draft() {
    use yo_core::{ActivityOutcome, ActivityQuestion, ActivityUpdate, QuestionChoice};
    for (supported, choice) in [(false, None), (true, None), (true, Some(1))] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(nonzero(95));
        let request = ActivityRequestRef::new(activity(1), request_id);
        let mut profile = ActivityQuestion {
            plain_text: "Second question".into(),
            choices: vec![QuestionChoice {
                label: "Layout".into(),
                description: "Readable output".into(),
            }],
            allow_notes: true,
            previous_question: supported,
            draft: Some("메모\n/exit".into()),
            draft_choice: choice,
        };
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::UserInputRequest { request_id },
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        assert_eq!(state.editor().text(), "메모\n/exit");
        let backwards = key(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert!(!matches!(
            state.handle(backwards.clone(), Duration::ZERO).unwrap(),
            StateEffect::Dispatch(_)
        ));
        for width in [80, 24, 80] {
            let frame = state
                .prepare_frame(Size::new(width, 30), &AppearanceState::default().pin())
                .unwrap();
            assert!(frame.overlay_presented);
            state.commit_frame(&frame);
            assert_eq!(state.editor().text(), "메모\n/exit");
        }
        if !supported {
            assert!(!matches!(
                state.handle(backwards, Duration::ZERO).unwrap(),
                StateEffect::Dispatch(_)
            ));
            assert_eq!(state.editor().text(), "메모\n/exit");
            continue;
        }
        assert_eq!(
            state.handle(backwards, Duration::ZERO).unwrap(),
            StateEffect::Dispatch(AgentAction::PreviousQuestion {
                request,
                choice,
                draft: "메모\n/exit".into()
            })
        );
        assert!(state.editor().text().is_empty());
        assert!(!matches!(
            state
                .handle(key(KeyCode::BackTab, KeyModifiers::SHIFT), Duration::ZERO)
                .unwrap(),
            StateEffect::Dispatch(_)
        ));
        let successor = RequestId::new(nonzero(96));
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(2),
                kind: ActivityKind::UserInputRequest {
                    request_id: successor,
                },
            })
            .unwrap();
        profile.previous_question = false;
        profile.draft = Some("previous draft".into());
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(2),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        assert_eq!(state.editor().text(), "previous draft");
        state
            .handle(InputEvent::Paste(" edited".into()), Duration::ZERO)
            .unwrap();
        let edited = state.editor().text().to_owned();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(2),
                update: ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            })
            .unwrap();
        assert_eq!(state.editor().text(), edited);
        if choice.is_none() {
            state
                .handle(
                    key(KeyCode::Character('u'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
            assert!(state.editor().text().is_empty());
            state
                .handle(InputEvent::Paste("/exit".into()), Duration::ZERO)
                .unwrap();
            assert!(!matches!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Dispatch(_)
            ));
            present_request(&mut state);
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Dispatch(AgentAction::RespondToUserInput {
                    request: ActivityRequestRef::new(activity(2), successor),
                    input: "/exit".into()
                })
            );
        }
    }
}
