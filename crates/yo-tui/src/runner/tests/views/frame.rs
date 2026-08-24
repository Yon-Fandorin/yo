use std::time::Duration;

use super::support::{function, observed_conversation, render_and_commit};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction},
    runner::state::{StateEffect, TuiState},
    surface::Size,
};

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

// header form은 문자 수 임계값이 아니라 실제 terminal cell 측정을 사용하므로 각 경계
// 바로 위와 아래에서도 key hint가 잘린 긴 form 대신 완전히 들어가는 다음 form을 택한다.
#[test]
fn header_forms_switch_at_measured_cell_width_boundaries() {
    let mut state = TuiState::new();
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();

    for (width, expected) in [
        (
            61,
            "Transcript · context - · F1 Chat · F2 Transcript · F3 Request",
        ),
        (60, "Transcript · F1 Chat · F2 Transcript · F3 Request"),
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
