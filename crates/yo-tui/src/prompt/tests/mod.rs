use std::time::Duration;

use super::{PromptFrame, PromptRenderError, PromptViewState, render};
use crate::{
    input::{
        editor::{EditorEffect, PromptEditor},
        event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    },
    surface::{CellContent, Color, Point, Rect, Size, Style, Surface},
};

mod failures;
mod scrolling;

fn editor_with(text: &str) -> PromptEditor {
    let mut editor = PromptEditor::new();
    assert_eq!(
        editor.handle(InputEvent::Paste(text.into()), false, Duration::default()),
        EditorEffect::BufferChanged
    );
    editor
}

fn move_cursor(editor: &mut PromptEditor, code: KeyCode, count: usize) {
    let event = InputEvent::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Press,
        state: KeyState::NONE,
    });

    for _ in 0..count {
        assert_eq!(
            editor.handle(event.clone(), false, Duration::default()),
            EditorEffect::BufferChanged
        );
    }
}

fn move_left(editor: &mut PromptEditor, count: usize) {
    move_cursor(editor, KeyCode::Left, count);
}

fn move_right(editor: &mut PromptEditor, count: usize) {
    move_cursor(editor, KeyCode::Right, count);
}

fn prompt_style() -> Style {
    Style {
        foreground: Color::Indexed(7),
        background: Color::Indexed(4),
        ..Style::default()
    }
}

fn rendered_text(surface: &Surface, y: u16) -> String {
    (0..surface.size().width)
        .filter_map(|x| match surface.cell(Point::new(x, y))?.content() {
            CellContent::Grapheme { text, .. } => Some(text.as_ref()),
            CellContent::Blank | CellContent::Continuation { .. } => None,
        })
        .collect()
}

// 검증된 layout은 grapheme footprint와 cursor를 Surface 좌표로 정확히 투영한다.
#[test]
fn projects_prompt_content_and_cursor() {
    let editor = editor_with("A가B");
    let style = prompt_style();
    let size = Size::new(4, 3);
    let mut state = PromptViewState::default();
    let mut surface = Surface::new(size).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&editor, &mut view, style, &mut state).unwrap()
    };

    assert_eq!(
        frame,
        PromptFrame {
            cursor: Point::new(0, 1),
            content_height: std::num::NonZeroU16::new(2).unwrap(),
            first_visible_row: 0,
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
