use super::{
    AgentShellViewState, TranscriptScrollCommand, TranscriptState, TranscriptViewMode, editor_with,
    id, render_into,
};
use crate::surface::Size;

// resize는 prompt 희망 높이를 유지하고 남는 transcript 영역의 tail window를 다시 계산한다.
#[test]
fn resize_reallocates_tracks_and_reflows_follow_tail() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "0\n1\n2\n3\n4\n5".into())
        .expect("unique user item");
    let editor = editor_with("a\nb");
    let mut state = AgentShellViewState::default();

    let (_, large) = render_into(&transcript, &editor, Size::new(6, 6), &mut state, None);
    let (_, small) = render_into(&transcript, &editor, Size::new(6, 4), &mut state, None);

    assert_eq!(large.transcript_area.size.height, 4);
    assert_eq!(small.transcript_area.size.height, 2);
    assert_eq!(small.prompt_area.size.height, 2);
    assert_eq!(
        small.transcript.unwrap().first_visible_row,
        small.transcript.unwrap().content_height - 2
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
