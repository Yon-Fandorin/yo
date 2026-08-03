use std::time::Duration;

use super::{
    AgentShellFrame, AgentShellRenderError, AgentShellRenderOptions, AgentShellStyles,
    AgentShellViewState, ShellChromeSnapshot, render, render_with_measure_hook,
};
use crate::{
    appearance::ActivityMotionFrame,
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    overlay::{
        PanelSnapshot, SelectionEntry, SelectionPanel, SelectionPanelAppearance,
        SelectionPanelGlyphs, SelectionPanelStyles,
    },
    prompt::{PromptGlyphs, PromptStyles},
    runner::PresentationMode,
    shell::ShellChromeStyles,
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
        chrome: ShellChromeStyles {
            activity: style(8),
            metrics: style(9),
            mode: style(10),
        },
        overlay: SelectionPanelAppearance {
            styles: SelectionPanelStyles {
                background: style(0),
                frame: style(11),
                title: style(12),
                hint: style(13),
                label: style(14),
                detail: style(15),
                selected: style(16),
                disabled: style(17),
            },
            glyphs: SelectionPanelGlyphs::rich(),
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

fn render_into_with_overlay(
    transcript: &TranscriptState,
    editor: &PromptEditor,
    size: Size,
    state: &mut AgentShellViewState,
    panel: &SelectionPanel,
    turn_active: bool,
) -> (Surface, AgentShellFrame) {
    let mut surface = Surface::new(size).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render_with_measure_hook(
            transcript,
            editor,
            &mut view,
            AgentShellRenderOptions {
                transcript_config: &TranscriptLayoutConfig::default(),
                styles: styles(),
                scroll: None,
                frame_prompt: size.height >= super::MIN_FRAMED_PROMPT_HEIGHT,
                chrome: ShellChromeSnapshot {
                    turn_active,
                    backend: Some("codex"),
                    workspace: "~/yo",
                    mode: PresentationMode::Inline,
                },
                activity_motion: ActivityMotionFrame::still("·"),
                overlay: Some(panel),
                overlay_bindings: &crate::overlay::OverlayBindings::default(),
            },
            state,
            || {},
        )
        .unwrap()
    };
    (surface, frame)
}

fn selection_panel() -> SelectionPanel {
    SelectionPanel::new(
        PanelSnapshot::new(
            "Commands",
            vec![
                SelectionEntry::enabled("one", "First command", Some("detail".into())),
                SelectionEntry::enabled("two", "Second command", None),
            ],
        )
        .unwrap(),
    )
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

    let (surface, frame) = render_into(&transcript, &editor, Size::new(8, 13), &mut state, None);

    assert_eq!(frame.transcript_area.size.height, 5);
    assert_eq!(frame.transient_area.origin.y, 5);
    assert_eq!(frame.transient_area.size.height, 2);
    assert_eq!(frame.prompt_area.origin.y, 7);
    assert_eq!(frame.prompt_area.size.height, 4);
    assert_eq!(frame.metrics_area.origin.y, 11);
    assert_eq!(frame.mode_area.origin.y, 12);
    assert_eq!(frame.cursor, Point::new(4, 9));
    assert_eq!(rendered_row(&surface, 0), "❯ q");
    assert_eq!(rendered_row(&surface, 2), "⏺ answer");
    assert_eq!(rendered_row(&surface, 7), "────────");
    assert_eq!(rendered_row(&surface, 8), "› ab");
    assert_eq!(rendered_row(&surface, 9), "  cd");
    assert_eq!(rendered_row(&surface, 10), "────────");
    assert_eq!(rendered_row(&surface, 11), "");
    assert_eq!(rendered_row(&surface, 12), "inline");
    assert_eq!(
        surface.cell(Point::new(2, 8)).unwrap().style(),
        styles().prompt.body
    );
    assert_eq!(
        surface.cell(Point::new(0, 8)).unwrap().style(),
        styles().prompt.marker
    );
    assert_eq!(
        surface.cell(Point::new(0, 7)).unwrap().style(),
        styles().prompt.rule
    );
}

// visible overlay는 기존 transcript tail과 transient work row 위에만 덮이며 prompt·metrics·mode
// 좌표는 같은 크기의 일반 frame과 동일하게 유지되고 Working 문구는 동시에 보이지 않는다.
#[test]
fn overlay_reuses_transcript_tail_and_work_row_without_relayout() {
    let transcript = TranscriptState::new();
    let editor = editor_with("draft");
    let panel = selection_panel();
    let size = Size::new(48, 13);
    let mut plain_state = AgentShellViewState::default();
    let mut overlay_state = AgentShellViewState::default();
    let (_, plain) = render_into(&transcript, &editor, size, &mut plain_state, None);
    let (surface, overlaid) =
        render_into_with_overlay(&transcript, &editor, size, &mut overlay_state, &panel, true);

    assert_eq!(overlaid.prompt_area, plain.prompt_area);
    assert_eq!(overlaid.metrics_area, plain.metrics_area);
    assert_eq!(overlaid.mode_area, plain.mode_area);
    assert_eq!(overlaid.cursor, plain.cursor);
    assert_eq!(
        overlaid.overlay_area.unwrap().end_y().unwrap(),
        overlaid.prompt_area.origin.y
    );
    assert!(rendered_row(&surface, overlaid.overlay_area.unwrap().origin.y).contains("Commands"));
    assert!(!(0..size.height).any(|y| rendered_row(&surface, y).contains("Working")));
    assert_eq!(
        rendered_row(&surface, overlaid.prompt_area.origin.y + 1),
        "› draft"
    );
    assert_eq!(
        rendered_row(&surface, overlaid.metrics_area.origin.y),
        "codex · ~/yo"
    );
}

// overlay 목적지가 border와 항목 한 행보다 작으면 panel을 숨기고 현재 활성 Turn의
// Working row를 다시 그려, 숨은 panel 때문에 상태 표시가 사라지지 않는다.
#[test]
fn hidden_overlay_restores_current_work_status() {
    let transcript = TranscriptState::new();
    let editor = editor_with("");
    let panel = selection_panel();
    let mut state = AgentShellViewState::default();

    let (surface, frame) = render_into_with_overlay(
        &transcript,
        &editor,
        Size::new(2, 4),
        &mut state,
        &panel,
        true,
    );

    assert_eq!(frame.overlay_area, None);
    assert_eq!(rendered_row(&surface, frame.transient_area.origin.y), "·");
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
