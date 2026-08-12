//! Preparation boundary for persistent Inline Chat rows and compact live geometry.

use super::chat::PublicationCandidate;
use crate::{
    appearance::{AppearanceRevision, AppearanceSnapshot},
    input::editor::PromptEditor,
    shell::{AgentShellMeasureError, AgentShellRenderOptions},
    surface::{Point, Rect, Size, Surface, SurfaceError},
    transcript::{
        TranscriptMeasureError, TranscriptRenderError, TranscriptSlice, TranscriptViewState,
        measure_slice, render_slice,
    },
};

pub(super) struct PreparedPublication {
    pub(super) surface: Surface,
    pub(super) observed_terminal_size: Size,
    pub(super) geometry_epoch: u64,
    pub(super) appearance_revision: AppearanceRevision,
    pub(super) candidate: PublicationCandidate,
}

#[derive(Debug)]
pub(super) enum PublicationPrepareError {
    Allocate(SurfaceError),
    Transcript(TranscriptRenderError),
}

impl PublicationPrepareError {
    pub(super) fn detail(&self) -> String {
        match self {
            Self::Allocate(error) => format!("allocating persistent rows failed: {error}"),
            Self::Transcript(error) => format!("rendering persistent rows failed: {error:?}"),
        }
    }
}

pub(super) fn compact_live_size(
    transcript: TranscriptSlice<'_>,
    editor: &PromptEditor,
    terminal_size: Size,
    options: AgentShellRenderOptions<'_>,
) -> Result<Size, AgentShellMeasureError> {
    let natural = crate::shell::natural_height(transcript, editor, terminal_size.width, options)?;
    Ok(Size::new(
        terminal_size.width,
        natural.min(terminal_size.height),
    ))
}

pub(super) fn prepare(
    transcript: TranscriptSlice<'_>,
    candidate: PublicationCandidate,
    observed_terminal_size: Size,
    geometry_epoch: u64,
    appearance_revision: AppearanceRevision,
    appearance: &AppearanceSnapshot,
) -> Result<PreparedPublication, PublicationPrepareError> {
    let height = measure_slice(
        transcript,
        observed_terminal_size.width,
        appearance.transcript_config(),
    )
    .map_err(|error| PublicationPrepareError::Transcript(render_error(error)))?
    .content_height;
    let mut surface = Surface::new(Size::new(observed_terminal_size.width, height))
        .map_err(PublicationPrepareError::Allocate)?;
    let mut view = surface
        .view(Rect::new(Point::new(0, 0), surface.size()))
        .expect("the complete publication Surface is a valid view");
    render_slice(
        transcript,
        &mut view,
        appearance.transcript_config(),
        appearance.styles().transcript,
        &mut TranscriptViewState::default(),
        None,
    )
    .map_err(PublicationPrepareError::Transcript)?;
    Ok(PreparedPublication {
        surface,
        observed_terminal_size,
        geometry_epoch,
        appearance_revision,
        candidate,
    })
}

fn render_error(error: TranscriptMeasureError) -> TranscriptRenderError {
    match error {
        TranscriptMeasureError::ZeroWidth => TranscriptRenderError::ZeroWidth,
        TranscriptMeasureError::InvalidConfig(error) => TranscriptRenderError::InvalidConfig(error),
        TranscriptMeasureError::BodyWidthUnavailable => TranscriptRenderError::BodyWidthUnavailable,
        TranscriptMeasureError::Text(error) => TranscriptRenderError::Text(error),
        TranscriptMeasureError::HeightOverflow => TranscriptRenderError::HeightOverflow,
    }
}
