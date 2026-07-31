use super::{
    TranscriptLayoutConfig, TranscriptScrollCommand, TranscriptState, TranscriptViewMode,
    TranscriptViewState, id, render_into, rendered_row,
};
use crate::surface::Size;

fn long_transcript() -> TranscriptState {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "0\n1\n2\n3\n4\n5\n6".into())
        .expect("unique user item");
    transcript
}

// 기본 FollowTail은 마지막 행을 붙잡고 새 content가 추가되어도 tail을 계속 보여준다.
#[test]
fn follows_the_tail_by_default() {
    let mut transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    let (surface, frame) = render_into(&transcript, Size::new(8, 3), &config, &mut state, None);
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(rendered_row(&surface, 0), "  4");
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);

    transcript
        .push_user(id(2), "tail".into())
        .expect("unique user item");
    let (surface, frame) = render_into(&transcript, Size::new(8, 3), &config, &mut state, None);
    assert_eq!(frame.first_visible_row, 7);
    assert_eq!(rendered_row(&surface, 2), "❯ tail");
}

// PageUp은 높이보다 한 행 적게 이동해 이전 화면의 첫 행을 다음 화면 끝에 남긴다.
#[test]
fn page_up_preserves_one_row_of_context() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    let (surface, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageUp),
    );

    assert_eq!(frame.first_visible_row, 2);
    assert_eq!(rendered_row(&surface, 0), "  2");
    assert_eq!(rendered_row(&surface, 2), "  4");
    assert_eq!(state.mode(), TranscriptViewMode::Detached);
}

// 아래로 tail까지 이동하면 FollowTail로 복귀해 이후 streaming 증가를 자동으로 따라간다.
#[test]
fn moving_down_to_the_tail_resumes_following() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageDown),
    );
    assert_eq!(frame.first_visible_row, 2);
    assert_eq!(state.mode(), TranscriptViewMode::Detached);

    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageDown),
    );
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
}

// 높이가 1인 화면도 PageDown을 0행이 아니라 1행 이동으로 해석한다.
#[test]
fn one_row_page_moves_by_one_line() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    render_into(
        &transcript,
        Size::new(8, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::PageDown),
    );

    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(state.mode(), TranscriptViewMode::Detached);
}

// 한 행 이동 명령과 tail 점프도 key가 아닌 동일한 의미 명령으로 상태를 전환한다.
#[test]
fn line_commands_and_tail_jump_share_the_semantic_scroll_path() {
    let transcript = long_transcript();
    let mut state = TranscriptViewState::default();
    let config = TranscriptLayoutConfig::default();

    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineUp),
    );
    assert_eq!(frame.first_visible_row, 3);
    assert_eq!(state.mode(), TranscriptViewMode::Detached);

    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineDown),
    );
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);

    render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    let (_, frame) = render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToTail),
    );
    assert_eq!(frame.first_visible_row, 4);
    assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
}

// 내용이 화면에 모두 보여 위로 이동할 수 없으면 no-op 명령이 tail 추적을 끊지 않는다.
#[test]
fn no_op_upward_commands_keep_following_short_content() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "short".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default();

    for command in [
        TranscriptScrollCommand::LineUp,
        TranscriptScrollCommand::PageUp,
    ] {
        let mut state = TranscriptViewState::default();
        let (_, frame) = render_into(
            &transcript,
            Size::new(8, 3),
            &config,
            &mut state,
            Some(command),
        );

        assert_eq!(frame.first_visible_row, 0);
        assert_eq!(state.mode(), TranscriptViewMode::FollowTail);
    }
}

// assistant 앞의 한 separator 행만 보이는 viewport는 아직 그 item을 표시하지 않았으므로
// 다음 item을 context로 잡지 않고, 실제 marker 행에 도달한 뒤에만 해당 ID를 보고한다.
#[test]
fn one_row_assistant_separator_has_no_following_item_context() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "first".into())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "second")
        .expect("streaming assistant");
    transcript.finalize(id(2)).expect("final assistant");
    let config = TranscriptLayoutConfig::default();
    let mut state = TranscriptViewState::default();

    let (separator, frame) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineUp),
    );
    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(rendered_row(&separator, 0), "");
    assert_eq!(frame.context_item, None);

    let (_, frame) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineDown),
    );
    assert_eq!(frame.first_visible_row, 2);
    assert_eq!(frame.context_item, Some(id(2)));
}

// user 앞의 두 separator 행은 각각 독립적인 한 행 viewport에서도 context가 없고,
// 이전·다음 item의 실제 marker 행만 정확한 item ID에 속한다.
#[test]
fn two_row_user_separator_excludes_both_blank_boundary_rows() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).expect("unique assistant");
    transcript
        .append_text(id(1), "first")
        .expect("streaming assistant");
    transcript.finalize(id(1)).expect("final assistant");
    transcript
        .push_user(id(2), "second".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default();
    let mut state = TranscriptViewState::default();

    let (_, first) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );
    assert_eq!(first.context_item, Some(id(1)));

    for expected_row in [1, 2] {
        let (separator, frame) = render_into(
            &transcript,
            Size::new(12, 1),
            &config,
            &mut state,
            Some(TranscriptScrollCommand::LineDown),
        );
        assert_eq!(frame.first_visible_row, expected_row);
        assert_eq!(rendered_row(&separator, 0), "");
        assert_eq!(frame.context_item, None);
    }

    let (_, second) = render_into(
        &transcript,
        Size::new(12, 1),
        &config,
        &mut state,
        Some(TranscriptScrollCommand::LineDown),
    );
    assert_eq!(second.first_visible_row, 3);
    assert_eq!(second.context_item, Some(id(2)));
}

// resize는 FollowTail을 새 tail에 맞추고 Detached offset은 clamp하되 읽기 의도는 유지한다.
#[test]
fn resize_reflows_following_and_clamps_detached_state() {
    let transcript = long_transcript();
    let config = TranscriptLayoutConfig::default();
    let mut following = TranscriptViewState::default();

    let (_, frame) = render_into(&transcript, Size::new(8, 4), &config, &mut following, None);
    assert_eq!(frame.first_visible_row, 3);
    assert_eq!(following.mode(), TranscriptViewMode::FollowTail);

    let mut detached = TranscriptViewState::default();
    render_into(
        &transcript,
        Size::new(8, 3),
        &config,
        &mut detached,
        Some(TranscriptScrollCommand::LineUp),
    );
    let (_, frame) = render_into(&transcript, Size::new(8, 8), &config, &mut detached, None);
    assert_eq!(frame.first_visible_row, 0);
    assert_eq!(detached.mode(), TranscriptViewMode::Detached);
}
