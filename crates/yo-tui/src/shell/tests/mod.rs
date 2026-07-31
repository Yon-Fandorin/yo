use std::time::Duration;

use super::{
    AgentShellFrame, AgentShellRenderError, AgentShellStyles, AgentShellViewState, render,
};
use crate::{
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    prompt::{PromptGlyphs, PromptStyles},
    surface::{CellContent, Color, Point, Rect, Size, Style, Surface},
    transcript::{
        TranscriptItemId, TranscriptLayoutConfig, TranscriptScrollCommand, TranscriptState,
        TranscriptStyles, TranscriptViewMode,
    },
};

mod failures;
mod resize;

fn id(value: u64) -> TranscriptItemId {
    TranscriptItemId::new(value)
}

fn editor_with(text: &str) -> PromptEditor {
    let mut editor = PromptEditor::new();
    if text.is_empty() {
        return editor;
    }
    assert_eq!(
        editor.handle(InputEvent::Paste(text.into()), false, Duration::default()),
        EditorEffect::BufferChanged
    );
    editor
}

fn style(index: u8) -> Style {
    Style {
        foreground: Color::Indexed(index),
        ..Style::default()
    }
}

fn styles() -> AgentShellStyles {
    AgentShellStyles {
        transcript: TranscriptStyles {
            background: style(0),
            user_marker: style(1),
            user_body: style(2),
            assistant_marker: style(3),
            assistant_body: style(4),
        },
        prompt: PromptStyles {
            body: style(5),
            marker: style(6),
            rule: style(7),
            glyphs: PromptGlyphs::rich(),
        },
    }
}

fn render_into(
    transcript: &TranscriptState,
    editor: &PromptEditor,
    size: Size,
    state: &mut AgentShellViewState,
    scroll: Option<TranscriptScrollCommand>,
) -> (Surface, AgentShellFrame) {
    let mut surface = Surface::new(size).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(
            transcript,
            editor,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            state,
            scroll,
        )
        .unwrap()
    };
    (surface, frame)
}

fn rendered_row(surface: &Surface, y: u16) -> String {
    (0..surface.size().width)
        .map(
            |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                CellContent::Blank | CellContent::Continuation { .. } => ' ',
                CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

// transcript는 남는 Flexible 영역을 쓰고 prompt는 본문 두 행과 위·아래 rule을 포함한
// Preferred 높이를 아래에 유지해 입력 컨테이너가 transcript와 분리되어 보이게 한다.
#[test]
fn composes_flexible_transcript_above_preferred_prompt() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "q".into())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "answer")
        .expect("streaming assistant");
    let editor = editor_with("ab\ncd");
    let mut state = AgentShellViewState::default();

    let (surface, frame) = render_into(&transcript, &editor, Size::new(8, 9), &mut state, None);

    assert_eq!(frame.transcript_area.size.height, 5);
    assert_eq!(frame.prompt_area.origin.y, 5);
    assert_eq!(frame.prompt_area.size.height, 4);
    assert_eq!(frame.cursor, Point::new(4, 7));
    assert_eq!(rendered_row(&surface, 0), "❯ q");
    assert_eq!(rendered_row(&surface, 2), "⏺ answer");
    assert_eq!(rendered_row(&surface, 5), "────────");
    assert_eq!(rendered_row(&surface, 6), "› ab");
    assert_eq!(rendered_row(&surface, 7), "  cd");
    assert_eq!(rendered_row(&surface, 8), "────────");
    assert_eq!(
        surface.cell(Point::new(2, 6)).unwrap().style(),
        styles().prompt.body
    );
    assert_eq!(
        surface.cell(Point::new(0, 6)).unwrap().style(),
        styles().prompt.marker
    );
    assert_eq!(
        surface.cell(Point::new(0, 5)).unwrap().style(),
        styles().prompt.rule
    );
}

// 한 행 화면에서는 prompt를 보장하고 숨겨진 transcript의 scroll 명령은 명시적인 no-op이다.
#[test]
fn one_row_shell_preserves_prompt_and_transcript_state() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "0\n1\n2\n3\n4".into())
        .expect("unique user item");
    let editor = editor_with("x");
    let mut state = AgentShellViewState::default();
    render_into(
        &transcript,
        &editor,
        Size::new(4, 4),
        &mut state,
        Some(TranscriptScrollCommand::LineUp),
    );
    assert_eq!(state.transcript.mode(), TranscriptViewMode::Detached);
    let before_state = state;

    let (surface, frame) = render_into(
        &transcript,
        &editor,
        Size::new(4, 1),
        &mut state,
        Some(TranscriptScrollCommand::JumpToStart),
    );

    assert_eq!(frame.transcript_area.size.height, 0);
    assert_eq!(frame.transcript, None);
    assert_eq!(frame.prompt_area.size.height, 1);
    assert_eq!(rendered_row(&surface, 0), "› x");
    assert_eq!(state, before_state);
}
