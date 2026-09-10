use super::{
    AgentShellViewState, TranscriptScrollCommand, TranscriptState, TranscriptViewMode, editor_with,
    id, render_into, rendered_row,
};
use crate::surface::Size;

// 큰 붙여넣기는 입력 viewport 안에서 스크롤하고 대화 영역과 실제 커서를 보존한다.
#[test]
fn long_draft_preserves_conversation_space_and_cursor() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "Keep this conversation visible".into())
        .unwrap();
    let draft = (0..30)
        .map(|n| format!("draft {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let editor = editor_with(&draft);
    let mut state = AgentShellViewState::default();
    let (surface, frame) = render_into(&transcript, &editor, Size::new(60, 24), &mut state, None);
    assert_eq!(frame.prompt_area.size.height, 8);
    assert!(frame.transcript_area.size.height >= 12);
    assert!(rendered_row(&surface, 0).contains("Keep this conversation"));
    assert!(rendered_row(&surface, frame.cursor.y).contains("draft 29"));
    assert!(frame.prompt.first_visible_row > 0);
    assert_eq!(editor.text(), draft);
}

// 과거 기록을 읽을 때만 위치와 최신 복귀 키를 보이고 End 이후 즉시 지운다.
#[test]
fn detached_history_has_a_position_and_return_hint() {
    let mut transcript = TranscriptState::new();
    transcript.push_user(id(1), "line\n".repeat(40)).unwrap();
    let editor = editor_with("");
    let mut state = AgentShellViewState::default();
    let (surface, frame) = render_into(
        &transcript,
        &editor,
        Size::new(60, 24),
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let hint = rendered_row(&surface, frame.transient_area.origin.y);
    assert!(hint.contains("History  1-"));
    assert!(hint.contains("End latest"));
    let (surface, frame) = render_into(
        &transcript,
        &editor,
        Size::new(60, 24),
        &mut state,
        Some(TranscriptScrollCommand::JumpToTail),
    );
    assert!(rendered_row(&surface, frame.transient_area.origin.y).is_empty());
}

// 충분히 큰 화면은 입력 본문과 chrome을 모두 예약하고, 작은 Chat 화면도 두 줄의
// transcript를 보존하며 남는 tail window를 다시 계산한다.
#[test]
fn resize_reallocates_tracks_and_reflows_follow_tail() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "0\n1\n2\n3\n4\n5".into())
        .expect("unique user item");
    let editor = editor_with("a\nb\nc");
    let mut state = AgentShellViewState::default();

    let (_, large) = render_into(&transcript, &editor, Size::new(6, 13), &mut state, None);
    let (small_surface, small) =
        render_into(&transcript, &editor, Size::new(6, 8), &mut state, None);

    assert_eq!(large.transcript_area.size.height, 4);
    assert_eq!(large.prompt_area.size.height, 5);
    assert_eq!(small.transcript_area.size.height, 2);
    assert_eq!(small.prompt_area.size.height, 3);
    assert_eq!(rendered_row(&small_surface, 3), "› a");
    assert_eq!(rendered_row(&small_surface, 4), "  b");
    assert_eq!(rendered_row(&small_surface, 5), "  c");
    assert_eq!(
        small.transcript.unwrap().first_visible_row,
        small.transcript.unwrap().content_height - 2
    );
    assert_eq!(state.transcript.mode(), TranscriptViewMode::FollowTail);
}

// 극단적으로 낮은 화면도 prompt 한 행과 transcript 한 행을 나눠 가져 footer detail은 숨긴다.
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

    assert_eq!(frame.transcript_area.size.height, 1);
    assert_eq!(frame.prompt_area.size.height, 1);
    assert_eq!(frame.prompt.first_visible_row, 2);
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
    assert_eq!(rendered_row(&compact_surface, 5), "›");
    assert_eq!(framed.prompt_area.size.height, 3);
    assert_eq!(rendered_row(&framed_surface, 4), "──────");
    assert_eq!(rendered_row(&framed_surface, 5), "›");
    assert_eq!(rendered_row(&framed_surface, 6), "──────");
}
