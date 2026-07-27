use super::{PromptRenderError, PromptViewState, editor_with, prompt_style, render};
use crate::{
    input::editor::{PromptEditor, layout::LayoutError},
    prompt::{PromptPaintError, paint_prepared, prepare},
    surface::{Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome},
};

fn scrolled_state() -> PromptViewState {
    let editor = editor_with("ab");
    let mut state = PromptViewState::default();
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let mut view = surface
        .view(Rect::new(Point::new(0, 0), Size::new(1, 1)))
        .unwrap();

    let frame = render(&editor, &mut view, prompt_style(), &mut state).unwrap();
    assert_eq!(frame.first_visible_row, 2);
    state
}

// 폭이 0인 view는 layout을 시도하거나 Surface와 스크롤 상태를 바꾸지 않는다.
#[test]
fn zero_width_preserves_surface_and_view_state() {
    let editor = editor_with("\u{301}");
    let mut state = scrolled_state();
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), Size::new(0, 1)))
            .unwrap();
        render(&editor, &mut view, prompt_style(), &mut state).unwrap_err()
    };

    assert_eq!(error, PromptRenderError::ZeroWidth);
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// 높이가 0인 view는 layout을 시도하거나 Surface와 스크롤 상태를 바꾸지 않는다.
#[test]
fn zero_height_preserves_surface_and_view_state() {
    let editor = editor_with("\u{301}");
    let mut state = scrolled_state();
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), Size::new(1, 0)))
            .unwrap();
        render(&editor, &mut view, prompt_style(), &mut state).unwrap_err()
    };

    assert_eq!(error, PromptRenderError::ZeroHeight);
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// 표시 정책이 없는 zero-width 입력은 layout 오류를 보존하고 상태를 부분 변경하지 않는다.
#[test]
fn layout_failure_preserves_surface_and_view_state() {
    let editor = editor_with("\u{301}");
    let size = Size::new(4, 1);
    let mut state = scrolled_state();
    let mut surface = Surface::new(size).unwrap();
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&editor, &mut view, prompt_style(), &mut state).unwrap_err()
    };

    assert_eq!(
        error,
        PromptRenderError::Layout(LayoutError::UnrenderableGrapheme {
            byte_index: 0,
            cause: crate::surface::GraphemeError::ZeroWidth
        })
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// view를 가로지르는 기존 wide footprint는 원자적으로 거절하고 상태를 그대로 보존한다.
#[test]
fn crossing_surface_footprint_preserves_surface_and_view_state() {
    let editor = PromptEditor::new();
    let mut state = scrolled_state();
    let mut surface = Surface::new(Size::new(3, 1)).unwrap();
    {
        let mut full = surface
            .view(Rect::new(Point::new(0, 0), Size::new(3, 1)))
            .unwrap();
        assert_eq!(
            full.write(
                Point::new(0, 0),
                Grapheme::try_from("가").unwrap(),
                prompt_style()
            ),
            WriteOutcome::Written
        );
    }
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut component = surface
            .view(Rect::new(Point::new(1, 0), Size::new(1, 1)))
            .unwrap();
        render(&editor, &mut component, Style::default(), &mut state).unwrap_err()
    };

    assert_eq!(error, PromptRenderError::SurfaceConflict);
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// 준비 폭과 다른 view는 paint 전에 거절되어 화면과 스크롤 상태를 건드리지 않는다.
#[test]
fn prepared_width_mismatch_is_rejected_before_painting() {
    let editor = editor_with("x");
    let prepared = prepare(&editor, 1).unwrap();
    let mut state = scrolled_state();
    let mut surface = Surface::new(Size::new(2, 1)).unwrap();
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), Size::new(2, 1)))
            .unwrap();
        paint_prepared(prepared, &mut view, prompt_style(), &mut state).unwrap_err()
    };

    assert_eq!(
        error,
        PromptPaintError::WidthMismatch {
            prepared: 1,
            actual: 2,
        }
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}
