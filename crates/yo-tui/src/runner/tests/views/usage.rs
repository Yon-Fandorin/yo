use std::time::Duration;

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, TranscriptRecord,
    session_repository::MANAGED_USAGE_SCHEMA,
};

use super::{
    super::{activity, key},
    support::{function, render_and_commit},
};
use crate::{
    appearance::AppearanceState,
    input::event::{KeyAction, KeyCode, KeyModifiers},
    runner::{
        state::{StateEffect, TuiState},
        view::ObservabilityView,
    },
    surface::{Attributes, Point, Size},
};

fn managed_receipt(activity_id: u64, provider: &str) -> String {
    format!(
        r#"{{"schema":"{MANAGED_USAGE_SCHEMA}","response_id":"response-{activity_id}","round":1,"provider":"{provider}","account":"team","model":"model","connector":"responses","api_dialect":"responses","base_url":"https://managed.invalid","usage":{{"input_tokens":100,"output_tokens":20,"total_tokens":120,"reasoning_tokens":5}},"cache_read_input_tokens":{{"availability":"reported","tokens":40,"source_profile":"managed.cache/v1"}}}}"#
    )
}

fn usage_records(count: u64) -> Vec<TranscriptRecord> {
    (1..=count)
        .flat_map(|activity_id| {
            let activity = activity(activity_id);
            [
                TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::ModelWork,
                }),
                TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(managed_receipt(
                        activity_id,
                        &format!("provider-{activity_id}"),
                    )),
                }),
                TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                }),
            ]
        })
        .collect()
}

fn observe_records(state: &mut TuiState, records: &[TranscriptRecord]) {
    for record in records {
        state.observe_record(record.clone()).unwrap();
    }
}

fn enter_usage(state: &mut TuiState) {
    assert_eq!(
        state.handle(function(4, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Usage);
}

// 영수증 목록의 모든 이동 명령은 선택 행과 viewport를 함께 갱신하고, 다른 view를 왕복해도
// 마지막 Usage 선택과 보이는 창을 그대로 복원한다.
#[test]
fn usage_navigation_and_view_switch_restore_selection_and_viewport() {
    let mut state = TuiState::new();
    observe_records(&mut state, &usage_records(8));
    enter_usage(&mut state);
    let size = Size::new(84, 12);
    render_and_commit(&mut state, size);

    for _ in 0..7 {
        assert_eq!(
            state.handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO),
            Ok(StateEffect::Redraw)
        );
        render_and_commit(&mut state, size);
    }
    assert_eq!(state.views().usage_position(), (7, 2));

    for (code, expected) in [
        (KeyCode::PageUp, (1, 1)),
        (KeyCode::PageDown, (7, 2)),
        (KeyCode::Home, (0, 0)),
        (KeyCode::End, (7, 2)),
    ] {
        state
            .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        render_and_commit(&mut state, size);
        assert_eq!(state.views().usage_position(), expected);
    }

    let before_switch = state.views().usage_position();
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(function(4, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    assert_eq!(state.views().usage_position(), before_switch);
}

// Usage에서 곧바로 F3 Request로 이동해도 Usage의 local transcript가 anchor로 잘못 조회되지
// 않고, Request가 문맥 없음 상태로 안전하게 렌더링된다.
#[test]
fn direct_usage_to_request_switch_renders_without_context_anchor() {
    let mut state = TuiState::new();
    observe_records(&mut state, &usage_records(1));
    enter_usage(&mut state);
    render_and_commit(&mut state, Size::new(72, 12));

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 12));

    assert!(request.contains("Request · context -"));
    assert!(request.contains("context_highlight=none(reason=no-viewed-journal-record)"));
}

// 영수증이 없는 Session은 명시적 빈 상태와 두 pane label을 보여 주며, 빈 목록에서 모든 local
// navigation을 소비해 editor로 새거나 상태를 어긋나게 하지 않는다.
#[test]
fn empty_usage_renders_labels_and_consumes_navigation() {
    let mut state = TuiState::new();
    enter_usage(&mut state);
    let size = Size::new(36, 14);
    let output = render_and_commit(&mut state, size);

    assert!(output.contains("receipts 0"));
    assert!(output.contains("No completed usage receipts."));
    assert!(output.contains("Receipts"));
    assert!(output.contains("Detail"));

    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
    ] {
        assert_eq!(
            state.handle(key(code, KeyModifiers::NONE), Duration::ZERO),
            Ok(StateEffect::Redraw)
        );
        render_and_commit(&mut state, size);
    }
    assert_eq!(state.views().usage_position(), (0, 0));
}

// summary가 body 높이보다 길어지는 2·3·4행 frame에서도 Usage는 빈 subview를 만들지 않고
// header와 이미 계산된 summary만 안전하게 잘라 렌더링한다.
#[test]
fn tiny_usage_frames_do_not_panic_when_summary_fills_body() {
    let mut state = TuiState::new();
    observe_records(&mut state, &usage_records(1));
    enter_usage(&mut state);

    for height in 2..=4 {
        let output = render_and_commit(&mut state, Size::new(36, height));
        assert!(output.lines().next().is_some());
    }
}

// 알려진 Usage schema의 malformed receipt는 typed projection 전체를 error 상태로 바꾸며, 부분
// totals나 receipt 목록을 함께 그리지 않고 명시적인 schema/detail을 표시한다.
#[test]
fn malformed_known_usage_renders_typed_error_without_partial_totals() {
    let activity = activity(1);
    let malformed = format!(
        r#"{{"schema":"{MANAGED_USAGE_SCHEMA}","response_id":"bad","round":1,"provider":"kimi","account":"team","model":"model","connector":"responses","api_dialect":"responses","base_url":"https://managed.invalid","usage":{{"input_tokens":"bad"}},"cache_read_input_tokens":{{"availability":"reported","tokens":0,"source_profile":"managed.cache/v1"}}}}"#
    );
    let records = [
        TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(malformed),
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }),
    ];
    let mut state = TuiState::new();
    observe_records(&mut state, &records);
    enter_usage(&mut state);

    let output = render_and_commit(&mut state, Size::new(72, 12));
    assert!(output.contains("Usage error"));
    assert!(output.contains(MANAGED_USAGE_SCHEMA));
    assert!(output.contains("input_tokens"));
    assert!(output.contains("token totals unavailable"));
    assert!(!output.contains("Tokens"));
    assert!(!output.contains("Receipts"));
}

// 넓은 Surface는 Receipts와 Detail을 좌우에 배치하고 기존 bold·dim style과 두 칸 gutter를
// 유지하며, 좁은 Surface는 같은 label을 세로로 쌓아 선택 행의 본문 열을 고정한다.
#[test]
fn usage_wide_and_narrow_surfaces_keep_layout_labels_and_styles() {
    let mut state = TuiState::new();
    observe_records(&mut state, &usage_records(3));
    enter_usage(&mut state);

    let wide_size = Size::new(84, 12);
    let wide = render_and_commit(&mut state, wide_size);
    assert!(wide.lines().any(|line| line.contains("Receipts")));
    assert!(wide.lines().any(|line| line.contains("Detail")));
    assert!(
        wide.lines()
            .any(|line| line.contains("[01]") && line.contains("Receipt 1/3"))
    );

    let wide_frame = state
        .prepare_frame(wide_size, &AppearanceState::default().pin())
        .unwrap();
    assert!(
        wide_frame
            .surface
            .cell(Point::new(0, 1))
            .unwrap()
            .style()
            .attributes
            .contains(Attributes::BOLD)
    );
    assert!(
        wide_frame
            .surface
            .cell(Point::new(0, 6))
            .unwrap()
            .style()
            .attributes
            .contains(Attributes::BOLD)
    );
    assert!(
        wide_frame
            .surface
            .cell(Point::new(42, 5))
            .unwrap()
            .style()
            .attributes
            .contains(Attributes::DIM)
    );
    state.commit_frame(&wide_frame);

    let narrow = render_and_commit(&mut state, Size::new(36, 16));
    assert!(narrow.lines().any(|line| line.contains("Receipts")));
    assert!(narrow.lines().any(|line| line.contains("Detail")));
    assert!(narrow.lines().any(|line| line.contains("Receipt 1/3")));
    assert!(!narrow.contains('│'));
}
