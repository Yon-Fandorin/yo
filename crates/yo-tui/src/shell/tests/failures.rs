use super::{
    AgentShellRenderError, AgentShellViewState, TranscriptLayoutConfig, TranscriptScrollCommand,
    TranscriptState, editor_with, id, render, render_into, styles,
};
use crate::{
    input::editor::layout::LayoutError,
    layout::vertical::VerticalLayoutError,
    prompt::PromptMeasureError,
    surface::{Grapheme, GraphemeError, Point, Rect, Size, Surface, WriteOutcome},
    text::flow::TextFlowError,
    transcript::TranscriptMeasureError,
};

fn detached_state() -> AgentShellViewState {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(99), "0\n1\n2\n3\n4".into())
        .expect("unique seed item");
    let editor = editor_with("");
    let mut state = AgentShellViewState::default();
    let (_, frame) = render_into(
        &transcript,
        &editor,
        Size::new(4, 4),
        &mut state,
        Some(TranscriptScrollCommand::LineUp),
    );
    assert!(
        frame
            .transcript
            .is_some_and(|frame| frame.first_visible_row > 0)
    );
    state
}

// prompt와 transcript가 동시에 잘못되어도 먼저 필요한 prompt 측정 오류를 결정적으로 반환한다.
#[test]
fn prompt_preflight_has_priority_without_mutation() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "\u{301}".into())
        .expect("unique user item");
    let editor = editor_with("\u{301}");
    let size = Size::new(4, 2);
    let mut surface = Surface::new(size).unwrap();
    let before_surface = surface.clone();
    let mut state = detached_state();
    let before_state = state;

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(
            &transcript,
            &editor,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            None,
        )
        .unwrap_err()
    };

    assert_eq!(
        error,
        AgentShellRenderError::PromptMeasure(PromptMeasureError::Layout(
            LayoutError::UnrenderableGrapheme {
                byte_index: 0,
                cause: GraphemeError::ZeroWidth,
            }
        ))
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// transcript 측정 실패도 shell clear 전에 반환되어 Surface와 두 view state를 보존한다.
#[test]
fn transcript_preflight_failure_is_atomic() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "\u{301}".into())
        .expect("unique user item");
    let editor = editor_with("");
    let size = Size::new(4, 2);
    let mut surface = Surface::new(size).unwrap();
    let before_surface = surface.clone();
    let mut state = detached_state();
    let before_state = state;

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(
            &transcript,
            &editor,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            None,
        )
        .unwrap_err()
    };

    assert_eq!(
        error,
        AgentShellRenderError::TranscriptMeasure(TranscriptMeasureError::Text(
            TextFlowError::UnrenderableGrapheme {
                byte_index: 0,
                cause: GraphemeError::ZeroWidth,
            }
        ))
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// transcript가 0행으로 접히면 숨겨진 내용의 오류가 보이는 prompt 렌더를 막지 않는다.
#[test]
fn hidden_transcript_defers_its_preflight_failure() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "\u{301}".into())
        .expect("unique user item");
    let editor = editor_with("x");
    let mut state = AgentShellViewState::default();

    let (surface, frame) = render_into(
        &transcript,
        &editor,
        Size::new(4, 1),
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );

    assert_eq!(frame.transcript, None);
    assert_eq!(super::rendered_row(&surface, 0), "› x");
}

// 높이 0은 prompt 최소 한 행을 만족하지 못하므로 layout 단계에서 원자적으로 거절한다.
#[test]
fn zero_height_shell_is_an_atomic_layout_failure() {
    let transcript = TranscriptState::new();
    let editor = editor_with("");
    let size = Size::new(4, 0);
    let mut surface = Surface::new(size).unwrap();
    let before_surface = surface.clone();
    let mut state = detached_state();
    let before_state = state;

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(
            &transcript,
            &editor,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            None,
        )
        .unwrap_err()
    };

    assert_eq!(
        error,
        AgentShellRenderError::VerticalLayout(VerticalLayoutError::InsufficientHeight {
            required: 1,
            available: 0,
        })
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// shell 바깥에서 시작한 wide footprint와 교차하면 전체 clear를 원자적으로 거절한다.
#[test]
fn outer_crossing_footprint_preserves_surface_and_state() {
    let transcript = TranscriptState::new();
    let editor = editor_with("");
    let mut surface = Surface::new(Size::new(4, 2)).unwrap();
    {
        let mut full = surface
            .view(Rect::new(Point::new(0, 0), Size::new(4, 2)))
            .unwrap();
        assert_eq!(
            full.write(
                Point::new(0, 0),
                Grapheme::try_from("가").unwrap(),
                styles().prompt.body
            ),
            WriteOutcome::Written
        );
    }
    let before_surface = surface.clone();
    let mut state = detached_state();
    let before_state = state;

    let error = {
        let mut shell = surface
            .view(Rect::new(Point::new(1, 0), Size::new(3, 2)))
            .unwrap();
        render(
            &transcript,
            &editor,
            &mut shell,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            None,
        )
        .unwrap_err()
    };

    assert_eq!(error, AgentShellRenderError::SurfaceConflict);
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}
