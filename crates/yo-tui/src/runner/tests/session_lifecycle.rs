use std::time::Duration;

use yo_core::{
    AgentCommand, AgentEvent, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind,
    TranscriptRecord, TurnOutcome, UserInput, WorkspaceReference,
};

use super::{key, turn};
use crate::{
    appearance::{AppearanceState, ColorCapability, MotionPreference},
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction, ExitReason, RunOutcome,
        session::TuiSession,
        state::{StateEffect, TuiState},
        unix::retained_session_output,
    },
};

// 종료용 출력은 저널에 확정된 Chat만 포함하고 아직 작성 중인 prompt는 섞지 않는다.
#[test]
fn session_output_contains_the_current_chat_without_the_prompt() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
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

// 화면의 u16 행 한계를 넘긴 사용자 메시지도 정상 종료 시 마지막 줄까지 보존한다.
#[test]
fn oversized_session_output_retains_text_after_successful_exit() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    retained
        .parts_mut()
        .state
        .handle(
            InputEvent::Paste(format!("{}END", "line\n".repeat(usize::from(u16::MAX) + 1))),
            Duration::ZERO,
        )
        .unwrap();
    retained
        .parts_mut()
        .state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from(format!(
                    "{}END",
                    "line\n".repeat(usize::from(u16::MAX) + 1)
                )),
            },
        ))
        .unwrap();

    let output = retained_session_output(&retained).unwrap();
    assert_eq!(output.matches("line").count(), usize::from(u16::MAX) + 1);
    assert!(output.ends_with("  END\n"));
}

// 비어 있는 prompt의 Ctrl+D는 runner가 정상 종료할 명시적인 effect다.
#[test]
fn empty_ctrl_d_requests_normal_exit() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('d'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
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

fn queue_message(state: &mut TuiState, text: &str) {
    state
        .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
}

// 후속 입력은 실행 중 steer로 보내지 않으며, 완료 뒤 하나씩 별도 Turn을 시작한다.
// Accepted가 TurnStarted보다 먼저 도착해도 두 번째 입력을 steer로 넘기지 않는다.
#[test]
fn follow_ups_wait_for_completion_and_do_not_clear_a_newer_identical_draft() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    queue_message(&mut state, "first");
    queue_message(&mut state, "second");
    assert_eq!(state.editor().text(), "");
    assert!(state.next_follow_up().unwrap().is_none());
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    let Some(AgentAction::Submit(first)) = state.next_follow_up().unwrap() else {
        panic!("first queued input must start a Turn");
    };
    assert_eq!(first.input().as_str(), "first");
    state
        .handle(InputEvent::Paste("first".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted { id: first.id() })
        .unwrap();
    assert_eq!(state.editor().text(), "first");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "first");
    assert!(state.next_follow_up().unwrap().is_none());
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    assert!(state.next_follow_up().unwrap().is_none());
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    let Some(AgentAction::Submit(second)) = state.next_follow_up().unwrap() else {
        panic!("second queued input must start after completion");
    };
    assert_eq!(second.input().as_str(), "second");
    assert_ne!(first.id(), second.id());
}

// 거절된 예약 입력은 소실되거나 자동 재시도되지 않는다. 명시적으로 재개하면 새 ID로
// 같은 snapshot을 다시 보내며, 접수 대기 중에는 중복 dispatch하지 않는다.
#[test]
fn rejected_follow_up_pauses_and_resumes_with_a_fresh_identity() {
    let mut state = TuiState::new();
    queue_message(&mut state, "retry me");
    let Some(AgentAction::Submit(first)) = state.next_follow_up().unwrap() else {
        panic!("queued input");
    };
    assert!(state.next_follow_up().unwrap().is_none());
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: first.id(),
            rejection: SubmissionRejection::new(SubmissionRejectionKind::Incompatible, "not ready"),
        })
        .unwrap();
    assert!(state.next_follow_up().unwrap().is_none());
    state
        .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
        .unwrap();
    let Some(AgentAction::Submit(retry)) = state.next_follow_up().unwrap() else {
        panic!("explicit resume");
    };
    assert_eq!(retry.input(), first.input());
    assert_ne!(retry.id(), first.id());
}

// 중단된 작업 뒤 예약 요청을 실행하지 않고, 빈 편집기로 회수해 수정하거나 버릴 수 있다.
#[test]
fn interrupted_follow_ups_pause_and_recall_without_overwriting_the_draft() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    queue_message(&mut state, "later");
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Interrupted,
        })
        .unwrap();
    assert!(state.next_follow_up().unwrap().is_none());
    state
        .handle(InputEvent::Paste("new draft".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor().text(), "new draft");
    state
        .handle(
            key(KeyCode::Character('u'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor().text(), "later");
    assert!(state.next_follow_up().unwrap().is_none());
}

// queue의 개수 및 UTF-8 byte 상한에서 첫 초과 입력은 잘라 넣거나 지우지 않는다.
#[test]
fn follow_up_limits_preserve_the_first_excess_draft() {
    let mut state = TuiState::new();
    for index in 0..16 {
        queue_message(&mut state, &format!("message {index}"));
    }
    queue_message(&mut state, "excess");
    assert_eq!(state.editor().text(), "excess");
    let Some(AgentAction::Submit(first)) = state.next_follow_up().unwrap() else {
        panic!("queue remains intact");
    };
    assert_eq!(first.input().as_str(), "message 0");
    let mut state = TuiState::new();
    let maximum = "é".repeat(32 * 1024);
    queue_message(&mut state, &maximum);
    assert_eq!(state.editor().text(), "");
    queue_message(&mut state, "x");
    assert_eq!(state.editor().text(), "x");
    let Some(AgentAction::Submit(full)) = state.next_follow_up().unwrap() else {
        panic!("maximum input retained");
    };
    assert_eq!(full.input().as_str(), maximum);
}

// 일반 입력 X의 접수가 끝나기 전에 같은 예약 X를 회수하지 않아, 수락 알림이 새로 회수한
// 메시지를 지우는 것을 막는다. 접수 후에는 예약 내용을 그대로 회수할 수 있다.
#[test]
fn pending_manual_admission_cannot_consume_a_recalled_follow_up() {
    let mut state = TuiState::new();
    queue_message(&mut state, "same");
    state
        .handle(InputEvent::Paste("same".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(manual)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("manual submission");
    };
    state
        .handle(
            key(KeyCode::Character('u'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor().text(), "");
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted { id: manual.id() })
        .unwrap();
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor().text(), "same");
}

// 일반 steer 거절도 사용자의 수정 지시가 적용되지 않은 상태이므로 남은 예약을 자동 실행하지 않는다.
#[test]
fn rejected_manual_steer_pauses_waiting_follow_ups() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    queue_message(&mut state, "later");
    state
        .handle(InputEvent::Paste("adjust".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Steer { submission, .. }) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("manual steer");
    };
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::Incompatible,
                "steer unavailable",
            ),
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    assert!(state.next_follow_up().unwrap().is_none());
    assert_eq!(state.editor().text(), "adjust");
}

// 실제 후보 선택의 신원은 거절·예약·회수·재제출을 거쳐 유지된다. 수락 뒤 같은 문자열을
// 직접 붙여 넣어도 과거 선택 신원을 되살려 파일 참조로 제출하지 않는다.
#[test]
fn selected_reference_survives_rejection_queue_and_recall_without_leaking_to_paste() {
    let (mut state, reference) = selected_workspace_prompt();
    for code in [KeyCode::Left, KeyCode::Right] {
        state
            .handle(key(code, KeyModifiers::CONTROL), Duration::ZERO)
            .unwrap();
    }
    let StateEffect::Dispatch(AgentAction::Submit(first)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("selected input submission")
    };
    assert_eq!(first.input().references().len(), 1);
    assert_eq!(
        first.input().references()[0].workspace_reference(),
        Some(&reference)
    );
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: first.id(),
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::StaleReference,
                "file unavailable",
            ),
        })
        .unwrap();
    assert_eq!(state.editor().text(), first.input().as_str());
    state
        .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
        .unwrap();
    assert!(state.editor().text().is_empty());
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor().text(), first.input().as_str());
    let StateEffect::Dispatch(AgentAction::Submit(retry)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("recalled input submission")
    };
    assert_eq!(retry.input(), first.input());
    assert_ne!(retry.id(), first.id());
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted { id: retry.id() })
        .unwrap();
    assert!(state.editor().text().is_empty());
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    state
        .handle(
            InputEvent::Paste(first.input().as_str().to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    state
        .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(literal)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("literal input submission")
    };
    assert_eq!(literal.input().as_str(), first.input().as_str());
    assert!(literal.input().references().is_empty());
}

// 선택된 경로를 단어 삭제 후 복원해도 문자열만 돌아오며 과거 파일 선택 권한은 되살리지 않는다.
#[test]
fn word_delete_and_yank_do_not_recreate_a_selected_reference() {
    let (mut state, _) = selected_workspace_prompt();
    let original = state.editor().text().to_owned();
    state
        .handle(
            key(KeyCode::Character('w'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    assert_ne!(state.editor().text(), original);
    state
        .handle(
            key(KeyCode::Character('y'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor().text(), original);
    state
        .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("restored literal input submission")
    };
    assert_eq!(submission.input().as_str(), original);
    assert!(submission.input().references().is_empty());
}

fn selected_workspace_prompt() -> (TuiState, WorkspaceReference) {
    use yo_core::{
        WorkspaceReferenceCandidate, WorkspaceReferenceKind, WorkspaceReferenceSearchStatus,
        WorkspaceReferenceSearchUpdate,
    };

    use crate::surface::Size;
    let mut state = TuiState::new();
    state.enable_workspace_references();
    let StateEffect::WorkspaceSearch(request) = state
        .handle(InputEvent::Paste("@src".into()), Duration::ZERO)
        .unwrap()
    else {
        panic!("workspace search")
    };
    let reference = WorkspaceReference::new(
        "file:unicode",
        "host:one",
        "workspace:one",
        "root:one",
        "src/한글 이름.rs",
        WorkspaceReferenceKind::File,
    )
    .unwrap();
    state.observe_workspace_reference_update(WorkspaceReferenceSearchUpdate::final_result(
        &request,
        WorkspaceReferenceSearchStatus::Complete,
        vec![WorkspaceReferenceCandidate::new(reference.clone())],
    ));
    let frame = state
        .prepare_frame(Size::new(60, 20), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    (state, reference)
}
