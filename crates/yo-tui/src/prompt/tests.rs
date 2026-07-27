use std::time::Duration;

use super::{PromptFrame, PromptRenderError, render};
use crate::{
    input::{
        editor::{EditorEffect, PromptEditor, layout::LayoutError},
        event::InputEvent,
    },
    surface::{CellContent, Color, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome},
};

fn editor_with(text: &str) -> PromptEditor {
    let mut editor = PromptEditor::new();
    assert_eq!(
        editor.handle(InputEvent::Paste(text.into()), false, Duration::default()),
        EditorEffect::BufferChanged
    );
    editor
}

fn prompt_style() -> Style {
    Style {
        foreground: Color::Indexed(7),
        background: Color::Indexed(4),
        ..Style::default()
    }
}

// 검증된 layout은 grapheme footprint와 cursor를 Surface 좌표로 정확히 투영한다.
#[test]
fn projects_prompt_content_and_cursor() {
    let editor = editor_with("A가B");
    let style = prompt_style();
    let size = Size::new(4, 3);
    let mut surface = Surface::new(size).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&editor, &mut view, style).unwrap()
    };

    assert_eq!(
        frame,
        PromptFrame {
            cursor: Point::new(0, 1),
            content_height: std::num::NonZeroU16::new(2).unwrap()
        }
    );
    assert!(matches!(
        surface.cell(Point::new(0, 0)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "A"
    ));
    assert!(matches!(
        surface.cell(Point::new(1, 0)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "가"
    ));
    assert!(matches!(
        surface.cell(Point::new(2, 0)).unwrap().content(),
        CellContent::Continuation { .. }
    ));
    assert!(matches!(
        surface.cell(Point::new(3, 0)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "B"
    ));
    assert_eq!(surface.cell(Point::new(3, 2)).unwrap().style(), style);
}

// 필요한 높이가 view보다 크면 기존 Surface를 지우지 않고 정확한 크기로 실패한다.
#[test]
fn insufficient_height_does_not_mutate_the_surface() {
    let editor = editor_with("A가B");
    let size = Size::new(2, 2);
    let mut surface = Surface::new(size).unwrap();
    {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        assert_eq!(
            view.write(
                Point::new(0, 0),
                Grapheme::try_from("Z").unwrap(),
                prompt_style()
            ),
            WriteOutcome::Written
        );
    }
    let before = surface.clone();

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&editor, &mut view, Style::default()).unwrap_err()
    };

    assert_eq!(
        error,
        PromptRenderError::InsufficientHeight {
            required: std::num::NonZeroU16::new(3).unwrap(),
            available: 2
        }
    );
    assert_eq!(surface, before);
}

// 폭이 0인 view는 layout을 시도하거나 기존 Surface를 바꾸지 않는다.
#[test]
fn zero_width_does_not_mutate_the_surface() {
    let editor = PromptEditor::new();
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let before = surface.clone();

    let error = {
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), Size::new(0, 1)))
            .unwrap();
        render(&editor, &mut view, prompt_style()).unwrap_err()
    };

    assert_eq!(error, PromptRenderError::ZeroWidth);
    assert_eq!(surface, before);
}

// 표시 정책이 없는 control 입력은 layout 오류를 보존하고 Surface를 부분 변경하지 않는다.
#[test]
fn layout_failure_does_not_mutate_the_surface() {
    let editor = editor_with("\t");
    let size = Size::new(4, 1);
    let mut surface = Surface::new(size).unwrap();
    let before = surface.clone();

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&editor, &mut view, prompt_style()).unwrap_err()
    };

    assert_eq!(
        error,
        PromptRenderError::Layout(LayoutError::UnrenderableGrapheme {
            byte_index: 0,
            cause: crate::surface::GraphemeError::Control
        })
    );
    assert_eq!(surface, before);
}

// view 경계를 가로지르는 기존 wide footprint는 원자적으로 거절하고 그대로 보존한다.
#[test]
fn crossing_surface_footprint_is_preserved_on_conflict() {
    let editor = PromptEditor::new();
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
    let before = surface.clone();

    let error = {
        let mut component = surface
            .view(Rect::new(Point::new(1, 0), Size::new(1, 1)))
            .unwrap();
        render(&editor, &mut component, Style::default()).unwrap_err()
    };

    assert_eq!(error, PromptRenderError::SurfaceConflict);
    assert_eq!(surface, before);
}
