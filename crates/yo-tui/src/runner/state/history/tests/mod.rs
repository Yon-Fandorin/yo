use std::{num::NonZeroU64, time::Duration};

use yo_core::{
    ActivityId, ActivityKind, ActivityRef, ActivityRequestRef, ActivityResponse, AgentCommand,
    AgentEvent, InputImage, InputImageSnapshot, InputReference, RequestId, SessionId,
    SkillReference, SkillReferenceScope, SubmissionOutcome, TranscriptRecord, TurnId, TurnRef,
    UserInput,
};

use super::{
    BYTE_LIMIT, ENTRY_LIMIT, InputEvent, KeyAction, KeyCode, KeyModifiers, PromptHistory,
    RestoredPromptHistory, StateEffect, TuiState,
};
use crate::{
    ColorCapability, MotionPreference,
    appearance::AppearanceState,
    input::event::{KeyEvent, KeyState},
    runner::{AgentAction, session::TuiSession},
    surface::Size,
};

fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

fn present(state: &mut TuiState) {
    let frame = state
        .prepare_frame(Size::new(60, 16), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
}

fn submit_and_accept(state: &mut TuiState, text: &str) {
    state
        .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("ordinary input must enter admission");
    };
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted {
            id: submission.id(),
        })
        .unwrap();
    state.starting_submission = None;
}

fn seed_resumed_history(state: &mut TuiState, input: UserInput) {
    state.install_restored_prompt_history(resumed_history(input));
}

fn resumed_history(input: UserInput) -> RestoredPromptHistory {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    RestoredPromptHistory::from_transcript_records(
        session_id,
        &[TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn { turn, input },
        )],
    )
}

// 33번째 입력은 가장 오래된 항목만 제거하고 최신 항목을 유지한다.
#[test]
fn evicts_oldest_after_entry_limit() {
    let mut history = PromptHistory::default();
    for index in 0..=ENTRY_LIMIT {
        history.retain(UserInput::new(index.to_string()));
    }
    assert_eq!(history.entries.len(), ENTRY_LIMIT);
    assert_eq!(history.recent()[0].as_str(), ENTRY_LIMIT.to_string());
    assert_eq!(history.recent()[ENTRY_LIMIT - 1].as_str(), "1");
}

// 용량을 넘는 한 입력은 기존의 복원 가능한 항목을 밀어내지 않는다.
#[test]
fn oversize_input_does_not_replace_history() {
    let mut history = PromptHistory::default();
    history.retain(UserInput::new("kept"));
    history.retain(UserInput::new("x".repeat(BYTE_LIMIT + 1)));
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.recent()[0].as_str(), "kept");
}

// 재개 seed는 현재 Session의 확정 StartTurn/SteerTurn만 복사하고, 다른 Session·질문 답변·비명령
// 기록과 oversized 입력은 기존 bounded history에 들어가지 않는다.
#[test]
fn resumed_seed_filters_records_before_copying_and_keeps_recent_bound() {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let parent_id: SessionId = "01890f00-0000-7000-8000-000000000002".parse().unwrap();
    let current_turn = |id| TurnRef::new(session_id, TurnId::new(NonZeroU64::new(id).unwrap()));
    let parent_turn = TurnRef::new(parent_id, TurnId::new(NonZeroU64::new(99).unwrap()));
    let records = [
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: current_turn(1),
            input: UserInput::new("ordinary start"),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::SteerTurn {
            turn: current_turn(2),
            input: UserInput::new("ordinary steer"),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
            request: ActivityRequestRef::new(
                ActivityRef::new(parent_turn, ActivityId::new(NonZeroU64::new(1).unwrap())),
                RequestId::new(NonZeroU64::new(1).unwrap()),
            ),
            response: ActivityResponse::UserInput(UserInput::new("activity answer")),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: parent_turn,
            input: UserInput::new("parent prompt"),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: current_turn(3),
            input: UserInput::new("ordinary start"),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: current_turn(4),
            input: UserInput::new("x".repeat(BYTE_LIMIT + 1)),
        }),
    ];
    let seed = RestoredPromptHistory::from_transcript_records(session_id, &records);
    let mut state = TuiState::new();
    state.install_restored_prompt_history(seed);

    assert_eq!(state.prompt_history.entries.len(), 3);
    let recent = state.prompt_history.recent();
    assert_eq!(recent[0].as_str(), "ordinary start");
    assert_eq!(recent[1].as_str(), "ordinary steer");
    assert_eq!(recent[2].as_str(), "ordinary start");

    // 재개 seed와 같은 입력을 live로 다시 승인해도 기존 반복 항목은 보존되고 새 승인 한 건만
    // 뒤에 추가된다.
    submit_and_accept(&mut state, "ordinary start");
    assert_eq!(state.prompt_history.entries.len(), 4);
    assert_eq!(state.prompt_history.recent()[0].as_str(), "ordinary start");
    assert_eq!(state.prompt_history.recent()[1].as_str(), "ordinary start");
}

// 재개 seed도 33번째 항목과 byte 한도를 한 단위 넘기는 입력에서 최신 항목을 남긴다.
#[test]
fn resumed_seed_keeps_newest_entries_at_both_capacity_boundaries() {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let records: Vec<_> = (1..=ENTRY_LIMIT + 1)
        .map(|id| {
            let turn = TurnRef::new(
                session_id,
                TurnId::new(NonZeroU64::new(u64::try_from(id).unwrap()).unwrap()),
            );
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
                turn,
                input: UserInput::new(id.to_string()),
            })
        })
        .collect();
    let seed = RestoredPromptHistory::from_transcript_records(session_id, &records);
    assert_eq!(seed.history.entries.len(), ENTRY_LIMIT);
    assert_eq!(
        seed.history.recent()[0].as_str(),
        (ENTRY_LIMIT + 1).to_string()
    );
    assert_eq!(seed.history.recent()[ENTRY_LIMIT - 1].as_str(), "2");

    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let records = [BYTE_LIMIT - 1, 1, 1].map(|len| {
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn,
            input: UserInput::new("x".repeat(len)),
        })
    });
    let seed = RestoredPromptHistory::from_transcript_records(session_id, &records);
    assert_eq!(seed.history.entries.len(), 2);
    assert_eq!(seed.history.bytes, 2);
    assert!(
        seed.history
            .recent()
            .iter()
            .all(|input| input.as_str() == "x")
    );
}

// public TuiSession builder는 live observation 전에 bounded resume seed를 상태에 설치한다.
#[test]
fn tui_session_builder_installs_resumed_prompt_history() {
    let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Reduced)
        .with_restored_prompt_history(resumed_history(UserInput::new("resumed")));
    let state = session.parts_mut().state;
    assert_eq!(state.prompt_history.entries.len(), 1);
    assert_eq!(state.prompt_history.recent()[0].as_str(), "resumed");
}

// 승인 전에는 이력이 없고, 선택 화면의 Enter는 복원만 수행한다. 다음 Enter만 재전송한다.
#[test]
fn accepted_prompt_recalls_for_editing_without_automatic_resend() {
    let mut state = TuiState::new();
    submit_and_accept(&mut state, "first\nsecond");
    assert_eq!(state.prompt_history.entries.len(), 1);
    assert_eq!(state.editor.text(), "");
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('r'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor.text(), "");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    present(&mut state);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor.text(), "first\nsecond");
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("separate Enter must submit restored prompt");
    };
    assert_eq!(submission.input().as_str(), "first\nsecond");
}

// 필터를 바꾼 직후의 낡은 frame은 선택할 수 없고, 취소하면 한글 초안과 커서가 복원된다.
#[test]
fn filtering_waits_for_fresh_frame_and_cancel_restores_draft_cursor() {
    let mut state = TuiState::new();
    submit_and_accept(&mut state, "alpha");
    state
        .handle(InputEvent::Paste("한글\n초안".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let saved = state.editor.clone();
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    present(&mut state);
    state
        .handle(InputEvent::Paste("alpha".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    state
        .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(state.editor, saved);
    assert_eq!(state.prompt_history.entries.len(), 1);
}

// 복원은 표시 marker뿐 아니라 실제 이미지 snapshot까지 다시 결합한다.
#[test]
fn image_prompt_recall_preserves_attachment_snapshot() {
    let snapshot: InputImageSnapshot = serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap();
    let input = UserInput::new("[image]")
        .with_images(vec![InputImage::new(0..7, 512, snapshot).unwrap()])
        .unwrap();
    let mut state = TuiState::new();
    seed_resumed_history(&mut state, input.clone());
    state
        .handle(
            key(KeyCode::Character('r'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    present(&mut state);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        input
    );
}

// 복원된 skill은 표시 문구만 남기는 대신 원래의 typed identity와 revision을 유지한다.
#[test]
fn skill_prompt_recall_preserves_reference_identity() {
    let skill = SkillReference::new(
        "skill:review",
        "host:one",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "sha256:exact",
    );
    let input =
        UserInput::with_references("$review inspect", vec![InputReference::skill(0..7, skill)])
            .unwrap();
    let mut state = TuiState::new();
    seed_resumed_history(&mut state, input.clone());
    state.open_recall_picker().unwrap();
    present(&mut state);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        input
    );
}

// 거절된 입력은 일반 메시지 이력에 들어가지 않는다.
#[test]
fn rejected_submission_is_not_recalled() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("rejected".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("ordinary input must enter admission");
    };
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: yo_core::SubmissionRejection::new(
                yo_core::SubmissionRejectionKind::StaleReference,
                "stale",
            ),
        })
        .unwrap();
    assert!(state.prompt_history.is_empty());
}

// 현재 panel을 연 뒤에도 새 이력이 생기면 보이는 첫 행은 여전히 같은 입력을 가리킨다.
#[test]
fn picker_selection_is_stable_when_history_changes() {
    let mut state = TuiState::new();
    state.prompt_history.retain(UserInput::new("older"));
    state.open_recall_picker().unwrap();
    present(&mut state);
    state.prompt_history.retain(UserInput::new("newer"));
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(state.editor.text(), "older");
}

// 질문 요청이 도착하면 검색 화면을 닫고 원래 초안을 되살린 뒤 요청 화면에 우선권을 준다.
#[test]
fn incoming_request_closes_picker_and_restores_original_draft() {
    let mut state = TuiState::new();
    state.prompt_history.retain(UserInput::new("old"));
    state
        .handle(InputEvent::Paste("new draft".to_owned()), Duration::ZERO)
        .unwrap();
    state.open_recall_picker().unwrap();
    let session_id = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let request_id = RequestId::new(NonZeroU64::new(1).unwrap());
    state
        .observe(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    assert!(state.recall_picker.is_none());
    assert_eq!(state.editor.text(), "new draft");
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('r'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Unchanged
    );
}

// 검색 결과의 숨은/조합 문자는 패널을 깨뜨리지 않고, Shift 문자도 필터에 들어간다.
#[test]
fn unusual_unicode_labels_and_shifted_filter_do_not_break_recall() {
    let mut state = TuiState::new();
    state
        .prompt_history
        .retain(UserInput::new("\u{200b}\u{301}Alpha"));
    state.open_recall_picker().unwrap();
    present(&mut state);
    state
        .handle(
            key(KeyCode::Character('A'), KeyModifiers::SHIFT),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(state.editor.text(), "A");
    present(&mut state);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(state.editor.text(), "\u{200b}\u{301}Alpha");
}

// 일반 메시지로 제출했던 slash 문구는 회수 뒤에도 로컬 명령이 되지 않는다.
#[test]
fn recalled_slash_text_remains_a_literal_prompt() {
    let mut state = TuiState::new();
    state.prompt_history.retain(UserInput::new("/quit"));
    state.open_recall_picker().unwrap();
    present(&mut state);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("recalled slash text must be sent as ordinary input");
    };
    assert_eq!(submission.input().as_str(), "/quit");
}

// 선택 화면에서도 Ctrl+C는 평소대로 원본 초안을 지우고 종료 시퀀스를 시작한다.
#[test]
fn ctrl_c_keeps_control_policy_during_recall() {
    let mut state = TuiState::new();
    state.prompt_history.retain(UserInput::new("old"));
    state.open_recall_picker().unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('c'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.recall_picker.is_none());
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('c'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Exit
    );
}

// 총량이 정확히 찼을 때 유지하고 첫 초과 바이트에서 가장 오래된 입력을 제거한다.
#[test]
fn aggregate_byte_limit_evicts_at_first_excess() {
    let mut history = PromptHistory::default();
    history.retain(UserInput::new("a".repeat(BYTE_LIMIT - 1)));
    history.retain(UserInput::new("b"));
    assert_eq!(history.entries.len(), 2);
    history.retain(UserInput::new("c"));
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.recent()[0].as_str(), "c");
    assert_eq!(history.recent()[1].as_str(), "b");
}

// 원본 초안 끝의 Ctrl+D는 텍스트를 바꾸지 않아도 검색 화면을 지운 frame을 요청한다.
#[test]
fn ctrl_d_closes_presented_picker_and_redraws_original_draft() {
    let mut state = TuiState::new();
    state.prompt_history.retain(UserInput::new("old"));
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();
    state.open_recall_picker().unwrap();
    present(&mut state);
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('d'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.recall_picker.is_none());
    assert_eq!(state.editor.text(), "draft");
}
