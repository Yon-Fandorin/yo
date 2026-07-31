use super::{
    AgentShellViewState, TranscriptScrollCommand, TranscriptState, TranscriptViewMode, editor_with,
    id, render_into,
};
use crate::surface::Size;

// 충분히 큰 화면은 입력 본문과 두 rule을 모두 예약하고, 작은 화면은 rule을 먼저 내려놓아
// 본문 두 행을 보존하며 남는 transcript의 tail window를 다시 계산한다.
#[test]
fn resize_reallocates_tracks_and_reflows_follow_tail() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "0\n1\n2\n3\n4\n5".into())
        .expect("unique user item");
    let editor = editor_with("a\nb\nc");
    let mut state = AgentShellViewState::default();

    let (_, large) = render_into(&transcript, &editor, Size::new(6, 9), &mut state, None);
    let (small_surface, small) =
        render_into(&transcript, &editor, Size::new(6, 4), &mut state, None);

    assert_eq!(large.transcript_area.size.height, 4);
    assert_eq!(large.prompt_area.size.height, 5);
    assert_eq!(small.transcript_area.size.height, 1);
    assert_eq!(small.prompt_area.size.height, 3);
    assert_eq!(super::rendered_row(&small_surface, 1), "› a");
    assert_eq!(super::rendered_row(&small_surface, 2), "  b");
    assert_eq!(super::rendered_row(&small_surface, 3), "  c");
    assert_eq!(
        small.transcript.unwrap().first_visible_row,
        small.transcript.unwrap().content_height - 1
    );
    assert_eq!(state.transcript.mode(), TranscriptViewMode::FollowTail);
}

// prompt가 화면보다 크면 Flexible transcript를 먼저 0으로 만들고 prompt를 현재 높이로 줄인다.
#[test]
fn constrained_height_shrinks_transcript_before_prompt() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "hidden".into())
        .expect("unique user item");
    let editor = editor_with("0\n1\n2");
    let mut state = AgentShellViewState::default();

    let (_, frame) = render_into(
        &transcript,
        &editor,
        Size::new(6, 2),
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );

    assert_eq!(frame.transcript_area.size.height, 0);
    assert_eq!(frame.prompt_area.size.height, 2);
    assert_eq!(frame.prompt.first_visible_row, 1);
}

// shell 높이 8은 compact prompt 한 행으로 transcript를 우선하고, 바로 다음 높이 9는
// 위·아래 rule 두 행을 예약해 frame 활성화 cutoff의 양쪽 결과를 직접 구분한다.
#[test]
fn shell_enables_prompt_frame_at_nine_rows() {
    let transcript = TranscriptState::new();
    let editor = editor_with("");
    let mut state = AgentShellViewState::default();

    let (compact_surface, compact) =
        render_into(&transcript, &editor, Size::new(6, 8), &mut state, None);
    let (framed_surface, framed) =
        render_into(&transcript, &editor, Size::new(6, 9), &mut state, None);

    assert_eq!(compact.prompt_area.size.height, 1);
    assert_eq!(super::rendered_row(&compact_surface, 7), "›");
    assert_eq!(framed.prompt_area.size.height, 3);
    assert_eq!(super::rendered_row(&framed_surface, 6), "──────");
    assert_eq!(super::rendered_row(&framed_surface, 7), "›");
    assert_eq!(super::rendered_row(&framed_surface, 8), "──────");
}
