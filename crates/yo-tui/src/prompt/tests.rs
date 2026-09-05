use std::time::Duration;

use super::{
    PromptFrame, PromptGlyphs, PromptRenderError, PromptStyles, PromptViewState, measure, render,
};
use crate::{
    input::{
        editor::{EditorEffect, PromptEditor},
        event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    },
    surface::{CellContent, Color, Point, Rect, Size, Style, Surface},
};

mod failures;
mod measurement;
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

fn prompt_styles() -> PromptStyles {
    PromptStyles {
        body: prompt_style(),
        marker: Style {
            foreground: Color::Indexed(3),
            ..Style::default()
        },
        rule: Style {
            foreground: Color::Indexed(8),
            ..Style::default()
        },
        glyphs: PromptGlyphs::rich(),
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

// Rich 입력 chrome은 위·아래 rule 사이 첫 행에 `› ` 2칸 prefix를 두고,
// 검증된 본문 grapheme과 cursor를 그 안쪽 좌표로 옮겨 입력창의 시각 계약을 보호한다.
#[test]
fn projects_prompt_content_and_cursor() {
    let editor = editor_with("A가B");
    let style = prompt_style();
    let size = Size::new(6, 4);
    let mut state = PromptViewState::default();
    let mut surface = Surface::new(size).unwrap();
    let measurement = measure(&editor, size.width).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&editor, &mut view, prompt_styles(), &mut state).unwrap()
    };

    assert_eq!(
        frame,
        PromptFrame {
            cursor: Point::new(2, 2),
            content_height: std::num::NonZeroU16::new(2).unwrap(),
            first_visible_row: 0,
        }
    );
    assert_eq!(
        measurement.desired_height,
        std::num::NonZeroU16::new(4).unwrap()
    );
    assert!(matches!(
        surface.cell(Point::new(0, 0)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "─"
    ));
    assert!(matches!(
        surface.cell(Point::new(0, 1)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "›"
    ));
    assert!(matches!(
        surface.cell(Point::new(2, 1)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "A"
    ));
    assert!(matches!(
        surface.cell(Point::new(3, 1)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "가"
    ));
    assert!(matches!(
        surface.cell(Point::new(4, 1)).unwrap().content(),
        CellContent::Continuation { .. }
    ));
    assert!(matches!(
        surface.cell(Point::new(5, 1)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "B"
    ));
    assert_eq!(surface.cell(Point::new(3, 2)).unwrap().style(), style);
}
