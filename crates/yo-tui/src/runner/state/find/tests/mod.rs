use std::{num::NonZeroU64, time::Duration};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, InputImage, InputImageSnapshot, InputReference, MessageContent, RequestId,
    SessionId, SkillReference, SkillReferenceScope, TranscriptRecord, TurnId, TurnOutcome, TurnRef,
    UserInput,
};

use super::*;
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    overlay::SelectionPanel,
    surface::Size,
    transcript::{TranscriptItemId, TranscriptState},
};

fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

fn present(state: &mut TuiState, size: Size) {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
}

fn assistant_item(transcript: &mut TranscriptState, id: u64, text: &str) {
    let id = TranscriptItemId::new(id);
    transcript.start_markdown_assistant(id).unwrap();
    transcript.append_text(id, text).unwrap();
    transcript.finalize(id).unwrap();
}

fn observe_assistant(state: &mut TuiState, text: &str) {
    let session: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(1).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    state.observe(AgentEvent::TurnStarted { turn }).unwrap();
    state
        .observe(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text.to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
}

fn install_rich_draft(state: &mut TuiState) -> UserInput {
    let skill = SkillReference::new(
        "skill:review",
        "host:one",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "sha256:exact",
    );
    let snapshot: InputImageSnapshot = serde_json::from_str(
        r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#,
    )
    .unwrap();
    let input = UserInput::with_references(
        "$review inspect\n[image]",
        vec![InputReference::skill(0..7, skill)],
    )
    .unwrap()
    .with_images(vec![InputImage::new(16..23, 512, snapshot).unwrap()])
    .unwrap();
    state.editor.replace_range(0..0, input.as_str());
    state
        .prompt_assist
        .restore_input(&input, &mut state.overlay);
    state.editor.handle(
        key(KeyCode::Left, KeyModifiers::NONE),
        false,
        Duration::ZERO,
    );
    input
}

// 최종 User와 Markdown Assistant만 검색하고 notice·구조화 활동·media snapshot은 숨긴다.
#[test]
fn corpus_includes_only_final_ordinary_user_and_assistant_messages() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(TranscriptItemId::new(1), "user needle".to_owned())
        .unwrap();

    let notice = TranscriptItemId::new(2);
    transcript.start_assistant(notice).unwrap();
    transcript.append_text(notice, "notice needle").unwrap();
    transcript.finalize(notice).unwrap();

    assistant_item(&mut transcript, 3, "assistant needle");

    let activity = TranscriptItemId::new(4);
    transcript
        .start_typed_activity_message(activity, ActivityKind::ModelWork)
        .unwrap();
    transcript
        .append_text(activity, "reasoning needle")
        .unwrap();
    transcript.finalize(activity).unwrap();

    let media = MessageContent {
        block: serde_json::json!({"mime_type": "image/png"}),
    }
    .to_snapshot()
    .unwrap();
    assistant_item(&mut transcript, 5, &media);

    let corpus = corpus(&transcript);
    assert_eq!(
        corpus
            .entries
            .iter()
            .map(|entry| entry.item.get())
            .collect::<Vec<_>>(),
        vec![3, 1]
    );
}

// corpus의 항목 수·총 바이트·개별 메시지·검사 항목 상한은 결과와 coverage 상태를 함께 제한한다.
#[test]
fn corpus_bounds_entries_bytes_message_size_and_examined_items() {
    let mut entries = TranscriptState::new();
    for id in 1..=CORPUS_ENTRY_LIMIT + 1 {
        entries
            .push_user(TranscriptItemId::new(id as u64), id.to_string())
            .unwrap();
    }
    let bounded = corpus(&entries);
    assert_eq!(bounded.entries.len(), CORPUS_ENTRY_LIMIT);
    assert!(bounded.truncated);
    assert_eq!(
        bounded.entries[0].item.get(),
        (CORPUS_ENTRY_LIMIT + 1) as u64
    );

    let mut bytes = TranscriptState::new();
    for id in 1..=9 {
        bytes
            .push_user(TranscriptItemId::new(id), "x".repeat(MESSAGE_BYTE_LIMIT))
            .unwrap();
    }
    let bounded = corpus(&bytes);
    assert_eq!(
        bounded.entries.len(),
        CORPUS_BYTE_LIMIT / MESSAGE_BYTE_LIMIT
    );
    assert!(bounded.truncated);

    let mut oversized = TranscriptState::new();
    oversized
        .push_user(TranscriptItemId::new(1), "x".repeat(MESSAGE_BYTE_LIMIT + 1))
        .unwrap();
    oversized
        .push_user(TranscriptItemId::new(2), "kept".to_owned())
        .unwrap();
    let bounded = corpus(&oversized);
    assert_eq!(bounded.entries.len(), 1);
    assert!(bounded.truncated);

    let mut notices = TranscriptState::new();
    for id in 1..=CORPUS_SCAN_LIMIT + 1 {
        let id = TranscriptItemId::new(id as u64);
        notices.start_assistant(id).unwrap();
        notices.append_text(id, "notice").unwrap();
        notices.finalize(id).unwrap();
    }
    let bounded = corpus(&notices);
    assert!(bounded.entries.is_empty());
    assert!(bounded.truncated);
}

// Assistant 결과는 notice가 같은 검색어를 포함해도 stable item ID로 선택된다.
#[test]
fn assistant_result_is_selectable_without_exposing_notice_text() {
    let mut state = TuiState::new();
    observe_assistant(&mut state, "assistant needle");
    state.chat.push_notice("notice needle".to_owned()).unwrap();

    state.open_find_picker("needle").unwrap();
    assert_eq!(
        state
            .overlay
            .panel()
            .and_then(|panel| panel.selected_identity())
            .map(|identity| identity.as_str()),
        Some("1")
    );
    present(&mut state, Size::new(60, 16));
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.find_picker.is_none());
    assert!(state.views.chat_has_pending_scroll());
}

// Find 취소는 다중 행 초안의 비끝 cursor와 prompt state를 보존하고 Ctrl+C/V를 소비한다.
#[test]
fn cancel_preserves_draft_cursor_and_consumes_ctrl_c_and_ctrl_v() {
    let mut state = TuiState::new();
    let expected_input = install_rich_draft(&mut state);
    let saved = state.editor.clone();
    let saved_input = state.prompt_assist.input(state.editor.text()).unwrap();
    state.open_find_picker("").unwrap();
    present(&mut state, Size::new(60, 16));

    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('v'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.find_picker.is_some());

    assert_eq!(
        state
            .handle(
                InputEvent::Key(KeyEvent {
                    code: KeyCode::Character('f'),
                    modifiers: KeyModifiers::CONTROL,
                    action: KeyAction::Release,
                    state: KeyState::NONE,
                }),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.find_picker.is_some());
    assert!(state.pending_image.is_none());

    state
        .handle(InputEvent::Paste("needle".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('c'), KeyModifiers::CONTROL),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.find_picker.is_none());
    assert_eq!(state.editor, saved);
    assert_eq!(
        state.prompt_assist.input(state.editor.text()).unwrap(),
        saved_input
    );
    assert_eq!(saved_input, expected_input);
}

// panel이 아직 화면에 확정되지 않았거나 너무 좁아 숨겨져도 Enter는 초안을 제출하지 않는다.
#[test]
fn enter_before_visible_find_frame_never_submits_the_saved_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("unsent draft".to_owned()), Duration::ZERO)
        .unwrap();
    let saved = state.editor.clone();
    state.open_find_picker("draft").unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.find_picker.is_some());

    let hidden = state
        .prepare_frame(Size::new(2, 4), &AppearanceState::default().pin())
        .unwrap();
    assert!(!hidden.overlay_presented);
    state.commit_frame(&hidden);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.find_picker.is_some());

    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor, saved);
}

// 새 승인 요청이 검색창을 닫을 때 검색어가 답변이나 원래 초안을 대체하지 않는다.
#[test]
fn incoming_request_closes_find_and_restores_the_unsent_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("unsent draft".to_owned()), Duration::ZERO)
        .unwrap();
    let saved = state.editor.clone();
    state.open_find_picker("").unwrap();
    state
        .handle(InputEvent::Paste("query".to_owned()), Duration::ZERO)
        .unwrap();

    let session: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(1).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    state
        .observe(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest {
                request_id: RequestId::new(NonZeroU64::new(1).unwrap()),
            },
        })
        .unwrap();

    assert!(state.find_picker.is_none());
    assert_eq!(state.editor, saved);
    assert!(state.has_pending_request());
}

// UTF-8 상한을 가로지르는 다중 바이트 입력은 앞선 문자 경계에서 안전하게 멈춘다.
#[test]
fn multibyte_query_paste_stops_before_the_first_excess_byte() {
    let crossing = format!("{}한", "a".repeat(QUERY_BYTE_LIMIT - 1));
    assert_eq!(bounded_query(&crossing), "a".repeat(QUERY_BYTE_LIMIT - 1));
    assert_eq!(sanitized_paste(&crossing), "a".repeat(QUERY_BYTE_LIMIT - 1));

    let mut state = TuiState::new();
    state.open_find_picker("").unwrap();
    assert_eq!(
        state
            .handle(InputEvent::Paste(crossing), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor.text(), "a".repeat(QUERY_BYTE_LIMIT - 1));
}

// 이미 published 된 과거 항목도 resize 뒤 전체 Chat surface에서 semantic jump한다.
#[test]
fn jumping_to_old_published_message_uses_full_resized_chat_surface() {
    let session: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let mut state = TuiState::new();
    for id in 1..=20 {
        let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(id).unwrap()));
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn,
                    input: UserInput::new(format!("history {id}")),
                },
            ))
            .unwrap();
    }
    let appearance = AppearanceState::default().pin();
    let published = state
        .prepare_frame_for_geometry(Size::new(32, 12), &appearance, Duration::ZERO, 1)
        .unwrap();
    assert!(published.publication.is_some());
    assert!(state.acknowledge_publication(&published));
    state.commit_frame(&published);

    state.open_find_picker("history 16").unwrap();
    present(&mut state, Size::new(32, 12));
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();

    let jumped = state
        .prepare_frame_for_geometry(Size::new(32, 6), &appearance, Duration::ZERO, 2)
        .unwrap();
    assert!(jumped.publication.is_none());
    assert_eq!(jumped.surface.size(), Size::new(32, 6));
    state.commit_frame(&jumped);
    assert!(state.views.view_positions().0 > 0);

    assert_eq!(
        state
            .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let tail = state
        .prepare_frame_for_geometry(Size::new(32, 6), &appearance, Duration::ZERO, 3)
        .unwrap();
    assert!(tail.publication.is_none());
    state.commit_frame(&tail);
    assert!(state.views.inline_publication_eligible());
}

// 결과 수·query byte·match 주변 preview는 bounded 상태를 유지한다.
#[test]
fn result_and_query_limits_are_bounded_and_preview_centers_match() {
    let entries = (1..=RESULT_LIMIT + 1)
        .map(|id| FindEntry {
            item: TranscriptItemId::new(id as u64),
            role: MessageRole::Assistant,
            text: format!("prefix {} needle suffix", "x".repeat(120)),
        })
        .collect::<Vec<_>>();
    let snapshot = FindPicker::panel(&entries, false, "needle");
    assert!(snapshot.offers(&RESULT_LIMIT.to_string()));
    assert!(!snapshot.offers(&(RESULT_LIMIT + 1).to_string()));
    let panel = SelectionPanel::new(snapshot);
    assert_eq!(panel.entries().len(), RESULT_LIMIT + 1);

    let query = bounded_query(&"é".repeat(QUERY_BYTE_LIMIT));
    assert!(query.len() <= QUERY_BYTE_LIMIT);
    assert!(query.is_char_boundary(query.len()));

    let snippet = preview(&format!("{} needle tail", "x".repeat(200)), "needle");
    assert!(snippet.contains("needle"));
    assert!(snippet.starts_with('…'));
}
