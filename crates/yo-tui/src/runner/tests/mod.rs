use std::{num::NonZeroU64, time::Duration};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate,
    AgentCommand, AgentEvent, AgentRuntime, ApprovalDecision, BackendEvent, BackendScriptStep,
    DurabilityGapCause, JournalDurability, RequestId, RuntimeError, RuntimePoll, ScriptedBackend,
    SubmissionId, TranscriptRecord, TurnId, TurnOutcome, TurnRef, UserInput,
    session_repository::DurableCutoff,
};

use super::{
    AgentAction, AgentConnection, AgentPoll, ExitReason, RunOutcome,
    session::TuiSession,
    state::{StateEffect, StateError, TuiState},
    unix::{drain_agent, handle_backpressured_input, prepare_resize, retained_session_output},
};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent as YoKeyEvent, KeyState},
    surface::{CellContent, Point, Size},
    terminal::mode::inline::{InlineFramePlan, InlineViewport},
};

mod appearance;
mod overlay;
mod reentry;
mod views;

fn key(code: KeyCode, modifiers: crate::input::event::KeyModifiers) -> InputEvent {
    InputEvent::Key(YoKeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

fn rendered_row(state: &TuiState, size: Size, y: u16) -> String {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    (0..size.width)
        .map(
            |x| match frame.surface.cell(Point::new(x, y)).unwrap().content() {
                CellContent::Blank | CellContent::Continuation { .. } => ' ',
                CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

// 입력 편집은 화면 상태만 바꾸고 아직 연결되지 않은 에이전트 응답을 만들지 않는다.
#[test]
fn edits_prompt_without_creating_transcript_items() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(InputEvent::Paste("질문".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "질문");
    assert!(state.transcript().items().is_empty());
}

// 저장 공간 압력의 구체적인 화면 표현은 별도 SOT가 소유한다. 이 단계에서는 typed cutoff를
// 잃지 않고 TUI 상태까지 전달해 이후 presenter가 Chat·status·banner 정책을 선택할 수 있다.
#[test]
fn retains_storage_pressure_for_a_future_presentation_policy() {
    let mut state = TuiState::new();
    let durability = JournalDurability::Gap {
        durable_cutoff: DurableCutoff::KnownEmpty,
        cause: DurabilityGapCause::Capacity,
    };

    assert_eq!(
        state.observe_durability(durability).unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.durability(), Some(durability));
}

// Enter는 immutable snapshot을 queue에 보내도 prompt를 즉시 비우지 않는다. 같은 ID의
// admission Accepted가 온 뒤에만 현재 draft를 비우고, Journal commit은 Chat에 한 번 나타난다.
#[test]
fn committed_submitted_prompt_becomes_one_user_transcript_item() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();

    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap()
    else {
        panic!("Enter should queue one immutable submission");
    };

    assert_eq!(submission.input().as_str(), "question");
    assert_eq!(state.editor().text(), "question");
    assert!(state.transcript().items().is_empty());

    state
        .observe_submission_outcome(yo_core::SubmissionOutcome::Accepted {
            id: submission.id(),
        })
        .unwrap();
    assert_eq!(state.editor().text(), "");

    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();

    assert_eq!(state.transcript().items().len(), 1);
    assert_eq!(rendered_row(&state, Size::new(12, 3), 0), "❯ question");
}

// admission이 진행되는 동안 사용자가 draft를 고치면 이전 snapshot의 Accepted는 그 새
// 편집을 지우지 않고, 같은 ID 결과를 다시 받아도 이미 소비한 snapshot을 건드리지 않는다.
#[test]
fn accepted_older_submission_preserves_a_newer_draft_and_ignores_duplicates() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("first".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap()
    else {
        panic!("Enter should queue the first snapshot");
    };
    state
        .handle(InputEvent::Paste(" revised".to_owned()), Duration::ZERO)
        .unwrap();

    let outcome = yo_core::SubmissionOutcome::Accepted {
        id: submission.id(),
    };
    assert_eq!(
        state.observe_submission_outcome(outcome.clone()).unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "first revised");
    assert_eq!(
        state.observe_submission_outcome(outcome).unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.editor().text(), "first revised");
}

// Rejected는 snapshot과 현재 editor를 모두 소비하지 않으며, 같은 draft의 연속 Enter는
// 첫 admission이 끝나기 전 중복 Backend 요청을 만들지 않는다.
#[test]
fn pending_duplicate_and_rejection_preserve_the_exact_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap()
    else {
        panic!("Enter should queue one snapshot");
    };
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );

    state
        .observe_submission_outcome(yo_core::SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: yo_core::SubmissionRejection::new(
                yo_core::SubmissionRejectionKind::StaleReference,
                "select the reference again",
            ),
        })
        .unwrap();

    assert_eq!(state.editor().text(), "question");
}

// 종료용 출력은 저널에 확정된 Chat만 포함하고 아직 작성 중인 prompt는 섞지 않는다.
#[test]
fn session_output_contains_the_current_chat_without_the_prompt() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();

    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();

    assert_eq!(output, "❯ question\n");
}

// 보존용 투영의 u16 행 한계를 넘겨도 이미 끝난 사용자 세션을 실패로 바꾸지 않고 출력을
// 생략한다.
#[test]
fn oversized_session_output_does_not_replace_a_successful_exit() {
    let mut retained = TuiSession::new();
    retained
        .parts_mut()
        .state
        .handle(
            InputEvent::Paste("\n".repeat(usize::from(u16::MAX) + 1)),
            Duration::ZERO,
        )
        .unwrap();
    retained
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap();
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("\n".repeat(usize::from(u16::MAX) + 1)),
            },
        ))
        .unwrap();

    assert_eq!(retained_session_output(&retained), None);
}

// Resize는 editor의 Ctrl+C 연속 입력 상태를 건드리지 않고 geometry effect로 분리된다.
#[test]
fn resize_is_forwarded_without_mutating_prompt_state() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(InputEvent::Resize(Size::new(120, 40)), Duration::ZERO)
            .unwrap(),
        StateEffect::Resize(Size::new(120, 40))
    );
    assert!(state.editor().text().is_empty());
}

// 일반 resize는 이전 frame을 근거로 같은 inline 영역에서 전체 재조정한다.
#[test]
fn resize_reconciles_the_owned_inline_viewport_with_the_previous_frame() {
    let old = Size::new(80, 3);
    let next = Size::new(100, 2);
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(old).commit();
    let previous = crate::surface::Surface::new(old).unwrap();
    let current = crate::surface::Surface::new(next).unwrap();
    let mut size = old;

    prepare_resize(&mut viewport, &mut size, next);

    assert_eq!(size, next);
    let pending = viewport.begin_frame(next);
    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reconcile {
            previous: old,
            current: next,
            owned_rows: old.height.max(next.height),
            previous_cursor: Point::new(0, old.height),
            cursor: Point::new(0, next.height),
        }
    );
    let diff = pending.diff(Some(&previous), &current).unwrap();
    assert_eq!(diff.previous_size(), old);
    assert_eq!(diff.current_size(), next);
    assert_eq!(diff.spans().len(), usize::from(next.height));
}

// 비어 있는 prompt의 Ctrl+D는 runner가 정상 종료할 명시적인 effect다.
#[test]
fn empty_ctrl_d_requests_normal_exit() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(
                key(
                    KeyCode::Character('d'),
                    crate::input::event::KeyModifiers::CONTROL,
                ),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
}

// 저널에 확정된 사용자 명령을 표시할 transcript ID가 더는 증가할 수 없으면 중복 ID로
// 일부만 넣지 않고 실패한다.
#[test]
fn item_id_overflow_preserves_empty_transcript() {
    let mut state = TuiState::new();
    state.set_next_item_id(u64::MAX);

    assert_eq!(
        state.observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("질문"),
            },
        )),
        Err(StateError::ItemIdOverflow)
    );
    assert!(state.transcript().items().is_empty());
}

// public outcome은 프로세스를 직접 종료하지 않고 정상 종료 이유를 반환한다.
#[test]
fn public_outcome_exposes_user_exit_reason() {
    assert_eq!(
        RunOutcome::user_requested(None).reason(),
        ExitReason::UserRequested
    );
}

// host 종료 요청은 OS signal identity를 노출하지 않고 별도 정상 종료 이유로 반환한다.
#[test]
fn public_outcome_exposes_host_termination_reason() {
    assert_eq!(
        RunOutcome::termination_requested(None).reason(),
        ExitReason::TerminationRequested
    );
}

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn turn() -> TurnRef {
    let session_id = "01890f00-0000-7000-8000-000000000001"
        .parse()
        .expect("the fixture is a UUIDv7");
    TurnRef::new(session_id, TurnId::new(nonzero(1)))
}

fn activity(value: u64) -> ActivityRef {
    ActivityRef::new(turn(), ActivityId::new(nonzero(value)))
}

// agent message의 streaming delta를 먼저 표시하더라도 final snapshot이 다르면 화면 문자열을
// authoritative 결과로 교체하고 완료 뒤 그대로 남긴다.
#[test]
fn renders_the_authoritative_agent_message_snapshot() {
    let mut state = TuiState::new();
    let message = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: message,
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextDelta("partial".to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextSnapshot("complete answer".to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: message,
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();

    assert_eq!(
        rendered_row(&state, Size::new(24, 3), 0),
        "• complete answer"
    );
}

// non-message Activity의 빈 delta는 label 뒤에 보이지 않는 줄 바꿈을 누적하지 않고
// transcript와 화면 revision을 그대로 유지한다.
#[test]
fn empty_activity_delta_does_not_add_placeholder_lines() {
    let mut state = TuiState::new();
    let tool = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    let before = state.transcript().clone();

    assert_eq!(
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: tool,
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap(),
        StateEffect::Unchanged
    );

    assert_eq!(state.transcript(), &before);
}

// tool과 file-change Activity는 agent message가 없어도 서로 다른 완료 관찰로 transcript에
// 계속 남아 코딩 작업이 chat text만으로 축소되지 않는다.
#[test]
fn retains_completed_tool_and_file_change_observations() {
    let mut state = TuiState::new();
    let tool = activity(1);
    let file = activity(2);
    for (activity, kind) in [
        (tool, ActivityKind::ToolCall),
        (file, ActivityKind::FileChange),
    ] {
        state
            .observe(AgentEvent::ActivityStarted { activity, kind })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }

    assert_eq!(
        rendered_row(&state, Size::new(30, 12), 0),
        "• Running tool…"
    );
    assert_eq!(
        rendered_row(&state, Size::new(30, 12), 2),
        "• File change observed"
    );
}

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
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
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
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "use the second option".to_owned(),
        })
    );
}

// TurnStarted 뒤 Ctrl+C는 process exit sequence를 시작하지 않고 해당 agent 작업의
// interrupt intent를 한 번 전달한다.
#[test]
fn active_turn_ctrl_c_dispatches_interrupt() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(
                    KeyCode::Character('c'),
                    crate::input::event::KeyModifiers::CONTROL,
                ),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// TurnStarted 뒤 Esc도 Ctrl+C와 같은 interrupt intent를 전달하며 종료 동작으로 새지 않는다.
#[test]
fn active_turn_escape_dispatches_interrupt() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Escape, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// command lane이 가득 찬 동안에도 runner의 제한 입력 경로는 Ctrl+C를 버리지 않고 활성
// Turn interrupt로 해석해 우선 control lane에 전달할 수 있게 한다.
#[test]
fn backpressure_still_services_active_turn_ctrl_c() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(
                KeyCode::Character('c'),
                crate::input::event::KeyModifiers::CONTROL,
            ),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// command lane이 가득 차도 활성 Turn의 Esc는 일반 입력처럼 버려지지 않고 control lane으로 간다.
#[test]
fn backpressure_still_services_active_turn_escape() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Escape, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// command lane이 가득 차도 빈 editor의 Ctrl+D는 보통 경로와 똑같이 즉시 정상 종료로
// 해석되어 provider stop과 terminal cleanup 경로에 도달한다.
#[test]
fn backpressure_still_services_empty_ctrl_d_exit() {
    let mut state = TuiState::new();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(
                KeyCode::Character('d'),
                crate::input::event::KeyModifiers::CONTROL,
            ),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Exit
    );
}

// Ctrl+Z의 최초 key press는 editor 내용이나 활성 Turn 여부와 무관하게 terminal 소유권
// 세대를 닫는 일시정지 요청으로 분리한다.
#[test]
fn ctrl_z_press_requests_terminal_suspension() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(
                    KeyCode::Character('z'),
                    crate::input::event::KeyModifiers::CONTROL,
                ),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Suspend
    );
    assert_eq!(state.editor().text(), "draft");
}

// enhanced keyboard protocol이 보내는 Ctrl+Z repeat와 release는 새 일시정지 요청으로
// 세지 않고 최초 press 하나만 경계 신호로 사용한다.
#[test]
fn ctrl_z_repeat_and_release_do_not_request_another_suspension() {
    for action in [KeyAction::Repeat, KeyAction::Release] {
        let mut state = TuiState::new();
        let input = InputEvent::Key(YoKeyEvent {
            code: KeyCode::Character('z'),
            modifiers: crate::input::event::KeyModifiers::CONTROL,
            action,
            state: KeyState::NONE,
        });

        assert_eq!(
            state.handle(input, Duration::ZERO).unwrap(),
            StateEffect::Unchanged
        );
    }
}

// Ctrl 외에 Shift나 Alt가 함께 눌린 변형은 shell job-control 명령으로 추측하지 않고
// editor의 미지원 입력으로 남긴다.
#[test]
fn modified_ctrl_z_is_not_treated_as_job_control() {
    for modifiers in [
        crate::input::event::KeyModifiers::CONTROL.union(crate::input::event::KeyModifiers::SHIFT),
        crate::input::event::KeyModifiers::CONTROL.union(crate::input::event::KeyModifiers::ALT),
    ] {
        let mut state = TuiState::new();

        assert_eq!(
            state
                .handle(key(KeyCode::Character('z'), modifiers), Duration::ZERO)
                .unwrap(),
            StateEffect::Unchanged
        );
    }
}

// command lane이 가득 차 있어도 Ctrl+Z는 일반 입력처럼 버려지지 않고 terminal 복구
// 경계까지 전달된다.
#[test]
fn backpressure_still_services_ctrl_z_suspension() {
    let mut state = TuiState::new();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(
                KeyCode::Character('z'),
                crate::input::event::KeyModifiers::CONTROL,
            ),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Suspend
    );
}

// normal command lane이 가득 차도 이미 관찰한 approval request의 입력과 Enter는 계속
// 처리되어 correlated response를 urgent lane으로 보낼 수 있다.
#[test]
fn backpressure_still_services_a_pending_approval_response() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(NonZeroU64::new(1).unwrap());
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    assert_eq!(
        handle_backpressured_input(
            &mut state,
            InputEvent::Paste("y".to_owned()),
            Duration::ZERO,
            true,
        )
        .unwrap(),
        StateEffect::Redraw
    );

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
            true,
        )
        .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(request_activity, request_id),
            decision: ApprovalDecision::Approved,
        })
    );
}

// 이미 다른 urgent control이 TUI 재시도 slot을 차지한 동안에는 다음 approval 입력을
// 소비하지 않아, state에서 request를 제거한 뒤 response를 잃는 상황을 만들지 않는다.
#[test]
fn backpressure_pauses_request_input_while_another_control_is_retained() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(NonZeroU64::new(1).unwrap());
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            InputEvent::Paste("y".to_owned()),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.has_pending_request());
}

// Turn의 실패 event는 활성 상태를 닫고 사용자에게 backend 오류 내용을 별도 transcript
// 항목으로 남긴다.
#[test]
fn renders_turn_failure_and_clears_active_state() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Failed(yo_core::Failure::new("provider stopped")),
        })
        .unwrap();

    assert!(!state.turn_active());
    assert_eq!(
        rendered_row(&state, Size::new(36, 3), 0),
        "• Turn failed: provider stopped"
    );
}

struct RuntimeConnection {
    runtime: AgentRuntime<ScriptedBackend>,
}

impl AgentConnection for RuntimeConnection {
    type Error = RuntimeError;

    fn dispatch(&mut self, _action: AgentAction) -> Result<super::DispatchOutcome, Self::Error> {
        unreachable!("this projection test consumes only backend observations")
    }

    fn retry(
        &mut self,
        _pending: super::PendingDispatch,
    ) -> Result<super::DispatchOutcome, Self::Error> {
        unreachable!("this projection test consumes only backend observations")
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(match self.runtime.poll_event()? {
            RuntimePoll::Pending => AgentPoll::Pending,
            RuntimePoll::Event(event) => AgentPoll::Record(TranscriptRecord::EventCommitted(event)),
            RuntimePoll::Closed => AgentPoll::Closed,
        })
    }
}

// ScriptedBackend의 coding Activity가 core 상관관계 검증을 통과한 뒤 TUI drain 경계에서
// Tool과 FileChange transcript로 함께 투영되는지 결합해 확인한다.
#[test]
fn projects_fake_backend_coding_activities_through_core_into_tui() {
    let active_turn = turn();
    let tool = activity(1);
    let file = activity(2);
    let create = AgentCommand::CreateSession {
        session_id: active_turn.session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::from("inspect"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: tool,
            update: ActivityUpdate::TextSnapshot("$ cargo test\nok".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: tool,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: file,
            kind: ActivityKind::FileChange,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: file,
            update: ActivityUpdate::TextSnapshot("update: src/lib.rs".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: file,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    let mut state = TuiState::new();
    for event in runtime.execute_command(create).unwrap() {
        state.observe(event).unwrap();
    }
    for event in runtime
        .execute_submission(start, SubmissionId::new().unwrap())
        .unwrap()
    {
        state.observe(event).unwrap();
    }
    let mut connection = RuntimeConnection { runtime };

    assert!(drain_agent(&mut connection, &mut state).unwrap());

    let frame = state
        .prepare_frame(Size::new(32, 18), &AppearanceState::default().pin())
        .unwrap();
    let rows = (0..18)
        .map(|y| {
            (0..32)
                .map(
                    |x| match frame.surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank | CellContent::Continuation { .. } => ' ',
                        CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
                    },
                )
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Running tool…"));
    assert!(rows.contains("$ cargo test"));
    assert!(rows.contains("File change observed"));
    assert!(rows.contains("update: src/lib.rs"));
}
