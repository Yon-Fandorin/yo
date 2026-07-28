use std::time::Duration;

use crate::{
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    shell::{self, AgentShellRenderError, AgentShellStyles, AgentShellViewState},
    surface::{Point, Rect, Size, Style, Surface, SurfaceError},
    transcript::{
        TranscriptItemId, TranscriptLayoutConfig, TranscriptState, TranscriptStateError,
        TranscriptStyles,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateEffect {
    Unchanged,
    Redraw,
    Exit,
    Resize(Size),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateError {
    Transcript(TranscriptStateError),
    ItemIdOverflow,
}

#[derive(Debug)]
pub(super) enum FrameError {
    Allocate(SurfaceError),
    Render(AgentShellRenderError),
}

impl FrameError {
    pub(super) fn detail(&self) -> String {
        match self {
            Self::Allocate(error) => format!("allocating the frame failed: {error}"),
            Self::Render(error) => format!("composing the agent shell failed: {error:?}"),
        }
    }
}

pub(super) struct PreparedFrame {
    pub(super) surface: Surface,
    pub(super) cursor: Point,
    view_state: AgentShellViewState,
}

#[derive(Debug, Default)]
pub(super) struct TuiState {
    transcript: TranscriptState,
    editor: PromptEditor,
    view: AgentShellViewState,
    next_item_id: u64,
}

impl TuiState {
    pub(super) fn new() -> Self {
        Self {
            next_item_id: 1,
            ..Self::default()
        }
    }

    pub(super) fn handle(
        &mut self,
        input: InputEvent,
        now: Duration,
    ) -> Result<StateEffect, StateError> {
        if let InputEvent::Resize(size) = input {
            return Ok(StateEffect::Resize(size));
        }

        let editor_before = self.editor.clone();
        let effect = self.editor.handle(input, false, now);
        match effect {
            EditorEffect::BufferChanged => Ok(StateEffect::Redraw),
            EditorEffect::Submitted(text) => {
                let id = TranscriptItemId::new(self.next_item_id);
                let Some(next) = self.next_item_id.checked_add(1) else {
                    self.editor = editor_before;
                    return Err(StateError::ItemIdOverflow);
                };
                if let Err(error) = self.transcript.push_user(id, text) {
                    self.editor = editor_before;
                    return Err(StateError::Transcript(error));
                }
                self.next_item_id = next;
                Ok(StateEffect::Redraw)
            },
            EditorEffect::Exit => Ok(StateEffect::Exit),
            EditorEffect::Unhandled
            | EditorEffect::NoChange
            | EditorEffect::ExitArmed
            | EditorEffect::InterruptTask => Ok(StateEffect::Unchanged),
        }
    }

    pub(super) fn prepare_frame(&self, size: Size) -> Result<PreparedFrame, FrameError> {
        let mut surface = Surface::new(size).map_err(FrameError::Allocate)?;
        let mut view_state = self.view;
        let frame = {
            let area = Rect::new(Point::new(0, 0), size);
            let mut view = surface
                .view(area)
                .expect("the complete surface is always a valid view");
            shell::render(
                &self.transcript,
                &self.editor,
                &mut view,
                &TranscriptLayoutConfig::default(),
                default_styles(),
                &mut view_state,
                None,
            )
            .map_err(FrameError::Render)?
        };

        Ok(PreparedFrame {
            surface,
            cursor: frame.cursor,
            view_state,
        })
    }

    pub(super) fn commit_frame(&mut self, frame: &PreparedFrame) {
        self.view = frame.view_state;
    }
}

const fn default_styles() -> AgentShellStyles {
    let style = Style::new(
        crate::surface::Color::Default,
        crate::surface::Color::Default,
        crate::surface::Attributes::empty(),
    );
    AgentShellStyles {
        transcript: TranscriptStyles {
            background: style,
            user_marker: style,
            user_body: style,
            assistant_marker: style,
            assistant_body: style,
        },
        prompt: style,
    }
}

#[cfg(test)]
impl TuiState {
    pub(super) fn transcript(&self) -> &TranscriptState {
        &self.transcript
    }

    pub(super) fn editor(&self) -> &PromptEditor {
        &self.editor
    }

    pub(super) fn set_next_item_id(&mut self, value: u64) {
        self.next_item_id = value;
    }
}
