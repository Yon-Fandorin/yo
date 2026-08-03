//! Agent shell composition of a flexible transcript and preferred prompt.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the shell lands before its application event loop consumer"
    )
)]

use crate::{
    appearance::ActivityMotionFrame,
    input::editor::PromptEditor,
    layout::vertical::VerticalLayoutError,
    overlay::{OverlayBindings, PanelPaintError, SelectionPanel},
    prompt::{
        PromptFrame, PromptMeasureError, PromptPaintError, PromptStyles, PromptViewState,
        paint_prepared as paint_prompt, prepare as prepare_prompt,
    },
    surface::{Point, Rect, SurfaceView, WriteOutcome},
    transcript::{
        TranscriptLayoutConfig, TranscriptMeasureError, TranscriptPaintError,
        TranscriptRenderFrame, TranscriptScrollCommand, TranscriptState, TranscriptStyles,
        TranscriptViewState, paint_prepared as paint_transcript, prepare as prepare_transcript,
    },
};

mod chrome;
pub(crate) use chrome::{ShellChromeSnapshot, ShellChromeStyles};

pub(crate) const MIN_FRAMED_PROMPT_HEIGHT: u16 = 9;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AgentShellViewState {
    transcript: TranscriptViewState,
    prompt: PromptViewState,
}

#[cfg(test)]
impl AgentShellViewState {
    pub(crate) const fn transcript_first_visible_row(self) -> u16 {
        self.transcript.first_visible_row()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgentShellStyles {
    pub(crate) transcript: TranscriptStyles,
    pub(crate) prompt: PromptStyles,
    pub(crate) chrome: ShellChromeStyles,
    pub(crate) overlay: crate::overlay::SelectionPanelAppearance,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentShellRenderOptions<'config> {
    pub(crate) transcript_config: &'config TranscriptLayoutConfig,
    pub(crate) styles: AgentShellStyles,
    pub(crate) scroll: Option<TranscriptScrollCommand>,
    pub(crate) frame_prompt: bool,
    pub(crate) chrome: ShellChromeSnapshot<'config>,
    pub(crate) activity_motion: ActivityMotionFrame<'config>,
    pub(crate) overlay: Option<&'config SelectionPanel>,
    pub(crate) overlay_bindings: &'config OverlayBindings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgentShellFrame {
    pub(crate) transcript_area: Rect,
    pub(crate) transient_area: Rect,
    pub(crate) prompt_area: Rect,
    pub(crate) metrics_area: Rect,
    pub(crate) mode_area: Rect,
    pub(crate) transcript: Option<TranscriptRenderFrame>,
    pub(crate) prompt: PromptFrame,
    pub(crate) cursor: Point,
    pub(crate) activity_motion_period: Option<std::time::Duration>,
    pub(crate) overlay_area: Option<Rect>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentShellRenderError {
    PromptMeasure(PromptMeasureError),
    TranscriptMeasure(TranscriptMeasureError),
    VerticalLayout(VerticalLayoutError),
    SurfaceConflict,
    TranscriptPaint(TranscriptPaintError),
    PromptPaint(PromptPaintError),
    OverlayPaint(PanelPaintError),
    Chrome(chrome::ShellChromeError),
}

pub(crate) fn render(
    transcript: &TranscriptState,
    editor: &PromptEditor,
    view: &mut SurfaceView<'_>,
    transcript_config: &TranscriptLayoutConfig,
    styles: AgentShellStyles,
    state: &mut AgentShellViewState,
    scroll: Option<TranscriptScrollCommand>,
) -> Result<AgentShellFrame, AgentShellRenderError> {
    render_with_measure_hook(
        transcript,
        editor,
        view,
        AgentShellRenderOptions {
            transcript_config,
            styles,
            scroll,
            frame_prompt: view.size().height >= MIN_FRAMED_PROMPT_HEIGHT,
            chrome: ShellChromeSnapshot {
                turn_active: false,
                backend: None,
                workspace: "",
                mode: crate::runner::PresentationMode::Inline,
            },
            activity_motion: ActivityMotionFrame::still("·"),
            overlay: None,
            overlay_bindings: &OverlayBindings::default(),
        },
        state,
        || {},
    )
}

pub(crate) fn render_with_measure_hook(
    transcript: &TranscriptState,
    editor: &PromptEditor,
    view: &mut SurfaceView<'_>,
    options: AgentShellRenderOptions<'_>,
    state: &mut AgentShellViewState,
    after_measure: impl FnOnce(),
) -> Result<AgentShellFrame, AgentShellRenderError> {
    let AgentShellRenderOptions {
        transcript_config,
        styles,
        scroll,
        frame_prompt,
        chrome,
        activity_motion,
        overlay,
        overlay_bindings,
    } = options;
    let size = view.size();
    let prompt = prepare_prompt(editor, size.width)
        .map_err(AgentShellRenderError::PromptMeasure)?
        .with_frame(frame_prompt);

    if size.height == 0 {
        return Err(AgentShellRenderError::VerticalLayout(
            VerticalLayoutError::InsufficientHeight {
                required: 1,
                available: 0,
            },
        ));
    }
    let shell_area = Rect::new(Point::new(0, 0), size);
    let layout = chrome::layout(shell_area, prompt.desired_height(), chrome.turn_active);
    let transcript_area = layout.transcript;
    let prompt_area = layout.prompt;
    let prepared_transcript = if transcript_area.size.height == 0 {
        None
    } else {
        Some(
            prepare_transcript(transcript, size.width, transcript_config)
                .map_err(AgentShellRenderError::TranscriptMeasure)?,
        )
    };
    let prepared_overlay = overlay.and_then(|panel| {
        panel.prepare(
            crate::surface::Size::new(size.width, layout.prompt.origin.y),
            styles.overlay,
            overlay_bindings,
            chrome.turn_active,
        )
    });

    after_measure();

    if view.clear(styles.transcript.background) == WriteOutcome::Clipped {
        return Err(AgentShellRenderError::SurfaceConflict);
    }

    let transcript_frame = if let Some(prepared) = prepared_transcript {
        let mut transcript_view = view
            .subview(transcript_area)
            .expect("vertical layout stays inside the shell view");
        Some(
            paint_transcript(
                prepared,
                &mut transcript_view,
                styles.transcript,
                &mut state.transcript,
                scroll,
            )
            .map_err(AgentShellRenderError::TranscriptPaint)?,
        )
    } else {
        None
    };

    let mut activity_motion_period = None;
    if prepared_overlay.is_none() && layout.transient.size.height > 0 {
        let mut transient = view
            .subview(layout.transient)
            .expect("chrome transient area stays inside the shell view");
        activity_motion_period =
            chrome::paint_transient(&mut transient, chrome, styles.chrome, activity_motion)
                .map_err(AgentShellRenderError::Chrome)?;
    }

    let overlay_area = if let Some(prepared) = prepared_overlay {
        let panel_size = prepared.size();
        let area = Rect::new(
            Point::new(0, layout.prompt.origin.y - panel_size.height),
            panel_size,
        );
        let mut overlay_view = view
            .subview(area)
            .expect("the bottom-anchored overlay stays above the prompt");
        prepared
            .paint(&mut overlay_view)
            .map_err(AgentShellRenderError::OverlayPaint)?;
        Some(area)
    } else {
        None
    };

    let prompt_frame = {
        let mut prompt_view = view
            .subview(prompt_area)
            .expect("vertical layout stays inside the shell view");
        paint_prompt(prompt, &mut prompt_view, styles.prompt, &mut state.prompt)
            .map_err(AgentShellRenderError::PromptPaint)?
    };
    let cursor = Point::new(
        prompt_area.origin.x + prompt_frame.cursor.x,
        prompt_area.origin.y + prompt_frame.cursor.y,
    );

    if layout.metrics.size.height > 0 {
        let mut metrics = view
            .subview(layout.metrics)
            .expect("chrome metrics area stays inside the shell view");
        chrome::paint_metrics(&mut metrics, chrome, styles.chrome.metrics)
            .map_err(AgentShellRenderError::Chrome)?;
    }
    if layout.mode.size.height > 0 {
        let mut mode = view
            .subview(layout.mode)
            .expect("chrome mode area stays inside the shell view");
        chrome::paint_mode(&mut mode, chrome, styles.chrome.mode)
            .map_err(AgentShellRenderError::Chrome)?;
    }

    Ok(AgentShellFrame {
        transcript_area,
        transient_area: layout.transient,
        prompt_area,
        metrics_area: layout.metrics,
        mode_area: layout.mode,
        transcript: transcript_frame,
        prompt: prompt_frame,
        cursor,
        activity_motion_period,
        overlay_area,
    })
}

#[cfg(test)]
mod tests;
