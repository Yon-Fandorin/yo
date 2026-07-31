use std::{num::NonZeroU64, time::Duration};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityUpdate, AgentCommand, AgentEvent,
    ApprovalDecision, RequestId, TranscriptRecord, UserInput,
};

use super::{activity, key, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    runner::{
        state::{StateEffect, TuiState},
        view::{ObservabilityView, RequestUnavailableReason},
    },
    surface::{CellContent, Point, Size, Surface},
};

fn rendered(surface: &Surface) -> String {
    let size = surface.size();
    (0..size.height)
        .map(|y| {
            (0..size.width)
                .map(
                    |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank | CellContent::Continuation { .. } => ' ',
                        CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
                    },
                )
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_and_commit(state: &mut TuiState, size: Size) -> String {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    let output = rendered(&frame.surface);
    state.commit_frame(&frame);
    output
}

fn function(number: u8, action: KeyAction) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code: KeyCode::Function(number),
        modifiers: KeyModifiers::NONE,
        action,
        state: KeyState::NONE,
    })
}

fn observed_conversation() -> TuiState {
    let mut state = TuiState::new();
    let tool = activity(1);
    for record in [
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("inspect the repository"),
        }),
        TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn: turn() }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity: tool,
            update: ActivityUpdate::TextSnapshot("cargo test -p yo-tui".to_owned()),
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity: tool,
            outcome: ActivityOutcome::Completed,
        }),
    ] {
        state.observe_record(record).unwrap();
    }
    state
}

// 같은 Journal 입력에서 Chat은 간결한 작업 표현을 유지하고 Transcript는 command/event와
// Activity 수명주기를 모두 보이며 Request는 마지막 정확한 문맥의 typed 부재를 표시한다.
#[test]
fn three_modes_render_distinct_visible_projections_from_one_journal() {
    let mut state = observed_conversation();
    let chat = render_and_commit(&mut state, Size::new(72, 12));
    assert!(chat.contains("Chat · context 4/5 · F1 Chat · F2 Transcript · F3 Request"));
    assert!(chat.contains("❯ inspect the repository"));
    assert!(chat.contains("⏺ Running tool…"));
    assert!(!chat.contains("event.activity_started"));

    assert_eq!(
        state.handle(function(2, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    let transcript = render_and_commit(&mut state, Size::new(72, 40));
    assert!(transcript.contains("Transcript · context 5/5 · F1 Chat · F2 Transcript · F3 Request"));
    assert!(transcript.contains("command.start_turn"));
    assert!(transcript.contains("event.activity_started"));
    assert!(transcript.contains("kind=tool_call"));
    assert!(transcript.contains("event.activity_finished"));
    assert!(transcript.contains("outcome=completed"));
    assert!(transcript.contains("[observation boundary]"));
    assert!(transcript.contains("JournalSequence"));
    assert!(transcript.contains("Request Audit detail"));

    assert_eq!(
        state.handle(function(3, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    let request = render_and_commit(&mut state, Size::new(72, 12));
    assert!(request.contains("Request · context 5/5 · F1 Chat · F2 Transcript · F3 Request"));
    assert!(request.contains("status: unavailable"));
    assert!(request.contains("reason: no_associated_request"));
    assert!(request.contains("nearby records were not selected"));
}

// 기본 binding은 press에서만 정확한 mode로 전환하고 이미 활성인 mode의 같은 키나 release는
// 상태를 바꾸지 않아 중복 transition을 만들지 않는다.
#[test]
fn function_keys_switch_exactly_and_idempotently() {
    let mut state = TuiState::new();
    assert_eq!(state.views().active(), ObservabilityView::Chat);

    assert_eq!(
        state.handle(function(2, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Transcript);
    assert_eq!(
        state.handle(function(2, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Unchanged)
    );
    assert_eq!(
        state.handle(function(2, KeyAction::Release), Duration::ZERO),
        Ok(StateEffect::Unchanged)
    );
    assert_eq!(
        state.handle(function(3, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Request);
    assert_eq!(
        state.handle(function(1, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Chat);
}

// Transcript와 Request가 활성일 때 paste와 Enter는 editor를 건드리거나 Submit/응답 dispatch를
// 만들지 않고 소비되며 Chat으로 돌아오면 기존 draft가 그대로 편집 가능하다.
#[test]
fn non_chat_modes_enforce_read_only_input_without_dispatch() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();

    for mode in [2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        assert_eq!(
            state.handle(InputEvent::Paste(" ignored".to_owned()), Duration::ZERO),
            Ok(StateEffect::Unchanged)
        );
        assert_eq!(
            state.handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO),
            Ok(StateEffect::Unchanged)
        );
        assert_eq!(state.editor().text(), "draft");
    }

    state
        .handle(function(1, KeyAction::Press), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state.handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO),
        Ok(StateEffect::Dispatch(crate::runner::AgentAction::Submit(
            "draft".to_owned()
        )))
    );
}

// 직접 request correlation이 없는 Journal 문맥은 no_associated_request이고, 정확한
// Activity request가 anchor인 문맥은 이 Slice에 Audit detail이 없다는 별도 typed 사유다.
#[test]
fn request_distinguishes_no_association_from_unavailable_audit_detail() {
    let mut no_request = observed_conversation();
    no_request
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    assert_eq!(
        no_request.views().request_reason(),
        RequestUnavailableReason::NoAssociatedRequest
    );

    let mut unavailable = TuiState::new();
    let request_id = RequestId::new(NonZeroU64::new(7).unwrap());
    unavailable
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: activity(3),
                kind: ActivityKind::ApprovalRequest { request_id },
            },
        ))
        .unwrap();
    render_and_commit(&mut unavailable, Size::new(72, 12));
    unavailable
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();

    assert_eq!(
        unavailable.views().request_reason(),
        RequestUnavailableReason::RequestAuditDetailUnavailable
    );
    let request = render_and_commit(&mut unavailable, Size::new(72, 12));
    assert!(request.contains("reason: request_audit_detail_unavailable"));
    assert!(request.contains("activity=3 request=7"));
    assert!(request.contains("exchange/revisions/attempts/redaction: unavailable"));
}

// Chat, Transcript, Request에서 각각 분리된 viewport를 움직인 뒤 mode를 왕복하면 같은
// anchor일 때 각 first-visible-row가 복원되어 다른 view의 scroll이 덮어쓰지 않는다.
#[test]
fn switching_restores_each_view_local_scroll_state() {
    let mut state = TuiState::new();
    for index in 0..12 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("question {index}")),
                },
            ))
            .unwrap();
    }
    let size = Size::new(12, 5);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);

    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    let detached = state.views().view_positions();
    assert!(detached.0 > 0);
    assert!(detached.1 > 0);
    assert!(detached.2 > 0);

    for mode in [1, 2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        render_and_commit(&mut state, size);
    }
    assert_eq!(state.views().view_positions(), detached);
}

// 폭 6의 좁은 terminal에서는 mode와 세 switching key를 한 줄 compact 표기로 유지하고,
// resize 뒤에도 Transcript full-page body가 prompt와 겹치지 않는다.
#[test]
fn narrow_and_resized_frames_keep_mode_chrome_and_full_page_layout() {
    let mut state = observed_conversation();
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();

    let narrow = render_and_commit(&mut state, Size::new(6, 5));
    assert_eq!(narrow.lines().next(), Some("[T]123"));
    assert!(!narrow.contains('›'));
    let one_row = render_and_commit(&mut state, Size::new(6, 1));
    assert_eq!(one_row, "[T]123");

    assert_eq!(
        state.handle(InputEvent::Resize(Size::new(24, 9)), Duration::ZERO),
        Ok(StateEffect::Resize(Size::new(24, 9)))
    );
    let resized = render_and_commit(&mut state, Size::new(24, 9));
    assert_eq!(resized.lines().next(), Some("Transcript · F1/F2/F3"));
    assert!(resized.contains("activity=1") || resized.contains("outcome=completed"));
    assert!(!resized.contains('›'));
}

// 같은 pinned appearance로 만든 세 mode frame은 header와 본문까지 동일 revision을 보고,
// mode 전환이 frame 중간에 별도 appearance를 읽는 경로를 만들지 않는다.
#[test]
fn all_modes_keep_one_appearance_revision_per_completed_frame() {
    let mut state = observed_conversation();
    let appearance = AppearanceState::default().pin();
    let revision = appearance.revision();

    for mode in [1, 2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        let frame = state.prepare_frame(Size::new(48, 10), &appearance).unwrap();
        assert_eq!(frame.appearance_revision, revision);
        state.commit_frame(&frame);
    }
}

// Request projection은 RespondToActivity의 정확한 ActivityRequestRef만 사용하므로 인접한
// 다른 request가 있어도 현재 anchor의 session/turn/activity/request identity를 바꾸지 않는다.
#[test]
fn request_anchor_never_falls_through_to_a_nearby_request() {
    let mut state = TuiState::new();
    let first = ActivityRequestRef::new(activity(4), RequestId::new(NonZeroU64::new(8).unwrap()));
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::RespondToActivity {
                request: first,
                response: yo_core::ActivityResponse::Approval(ApprovalDecision::Approved),
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(AgentEvent::TurnStarted {
            turn: turn(),
        }))
        .unwrap();
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, Size::new(72, 10));
    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();

    let request = render_and_commit(&mut state, Size::new(72, 10));
    assert!(request.contains("reason: no_associated_request"));
    assert!(request.contains("event.turn_started"));
    assert!(!request.contains("request=8"));
}

// Chat에 보이지 않는 TurnStarted가 뒤에 추가되어도 마지막으로 실제 표시된 approval
// item의 문맥을 유지하므로 Request anchor가 숨은 Journal record로 밀리지 않는다.
#[test]
fn chat_context_ignores_hidden_records_after_the_visible_item() {
    let mut state = TuiState::new();
    let request_id = RequestId::new(NonZeroU64::new(13).unwrap());
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: activity(6),
                kind: ActivityKind::ApprovalRequest { request_id },
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(AgentEvent::TurnStarted {
            turn: turn(),
        }))
        .unwrap();

    let chat = render_and_commit(&mut state, Size::new(40, 8));
    assert!(chat.contains("Chat · F1/F2/F3"));
    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));

    assert!(request.contains("anchor: observed record #1 (event.activity_started)"));
    assert!(request.contains("activity=6 request=13"));
    assert!(!request.contains("event.turn_started"));
}

// approval item의 완료 처리는 phase만 finalize하고 보이는 text를 바꾸지 않으므로 최초
// request correlation을 ActivityFinished record로 덮지 않고 정확한 Request anchor를 보존한다.
#[test]
fn lifecycle_only_finish_preserves_visible_request_correlation() {
    let mut state = TuiState::new();
    let request_id = RequestId::new(NonZeroU64::new(17).unwrap());
    let request_activity = activity(10);
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: request_activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                activity: request_activity,
                outcome: ActivityOutcome::Completed,
            },
        ))
        .unwrap();

    render_and_commit(&mut state, Size::new(48, 8));
    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));

    assert!(request.contains("anchor: observed record #1 (event.activity_started)"));
    assert!(request.contains("activity=10 request=17"));
    assert!(!request.contains("event.activity_finished"));
}

// 고빈도 delta는 매 record마다 전체 Chat item을 복사·재검색하지 않고 변경된 단일 item만
// typed signal로 갱신하므로 512회 streaming 뒤에도 context map은 한 항목으로 제한된다.
#[test]
fn high_frequency_stream_updates_one_local_chat_context_entry() {
    let mut state = TuiState::new();
    let stream = activity(11);
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: stream,
                kind: ActivityKind::AgentMessage,
            },
        ))
        .unwrap();
    for _ in 0..512 {
        state
            .observe_record(TranscriptRecord::EventCommitted(
                AgentEvent::ActivityUpdated {
                    activity: stream,
                    update: ActivityUpdate::TextDelta("x".to_owned()),
                },
            ))
            .unwrap();
    }

    assert_eq!(state.views().chat_context_count(), 1);
    let chat = render_and_commit(&mut state, Size::new(20, 5));
    assert!(chat.contains("Chat · F1/F2/F3"));
}

// 줄바꿈된 Chat item에서 한 줄 위로 이동하면 record 수가 아니라 실제 viewport 행으로
// 문맥을 계산하여 화면 상단의 긴 user item을 정확히 anchor한다.
#[test]
fn wrapped_chat_rows_map_to_the_visible_record_context() {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("wrapped question ".repeat(8)),
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: activity(7),
                kind: ActivityKind::ApprovalRequest {
                    request_id: RequestId::new(NonZeroU64::new(14).unwrap()),
                },
            },
        ))
        .unwrap();
    let size = Size::new(18, 6);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let chat = render_and_commit(&mut state, size);
    assert!(chat.contains("Chat · F1/F2/F3"));

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));
    assert!(request.contains("anchor: observed record #1 (command.start_turn)"));
    assert!(request.contains("reason: no_associated_request"));
    assert!(!request.contains("request=14"));
}

// Transcript page 이동은 wrapped record의 실제 행 범위를 사용하므로 양옆 request가
// 있어도 가운데 보이는 StartTurn만 선택하고 이웃 correlation을 빌리지 않는다.
#[test]
fn transcript_page_movement_uses_visible_rows_and_not_neighbor_records() {
    let mut state = TuiState::new();
    for record in [
        TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
            request: ActivityRequestRef::new(
                activity(8),
                RequestId::new(NonZeroU64::new(15).unwrap()),
            ),
            response: yo_core::ActivityResponse::Approval(ApprovalDecision::Approved),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("middle wrapped record ".repeat(8)),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
            request: ActivityRequestRef::new(
                activity(9),
                RequestId::new(NonZeroU64::new(16).unwrap()),
            ),
            response: yo_core::ActivityResponse::Approval(ApprovalDecision::Declined),
        }),
    ] {
        state.observe_record(record).unwrap();
    }
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let size = Size::new(24, 7);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let transcript = render_and_commit(&mut state, size);
    assert!(transcript.contains("Transcript · F1/F2/F3"));

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));
    assert!(request.contains("anchor: observed record #2 (command.start_turn)"));
    assert!(request.contains("reason: no_associated_request"));
    assert!(!request.contains("request=15"));
    assert!(!request.contains("request=16"));
}

// header form은 문자 수 임계값이 아니라 실제 terminal cell 측정을 사용하므로 각 경계
// 바로 위와 아래에서도 key hint가 잘린 긴 form 대신 완전히 들어가는 다음 form을 택한다.
#[test]
fn header_forms_switch_at_measured_cell_width_boundaries() {
    let mut state = TuiState::new();
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();

    for (width, expected) in [
        (49, "Transcript · F1 Chat · F2 Transcript · F3 Request"),
        (48, "Transcript · F1/F2/F3"),
        (21, "Transcript · F1/F2/F3"),
        (20, "[T]123"),
        (6, "[T]123"),
        (5, "T123"),
        (4, "T123"),
        (3, "[T]"),
        (2, "T"),
        (1, "T"),
    ] {
        let frame = render_and_commit(&mut state, Size::new(width, 1));
        assert_eq!(frame, expected);
    }
}
