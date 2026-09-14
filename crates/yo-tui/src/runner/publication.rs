//! Preparation boundary for persistent Inline Chat rows and compact live geometry.

use super::chat::PublicationCandidate;
use crate::{
    appearance::{AppearanceRevision, AppearanceSnapshot},
    input::editor::PromptEditor,
    shell::{AgentShellMeasureError, AgentShellRenderOptions, natural_height},
    surface::{Point, Rect, Size, Surface, SurfaceError, WriteOutcome},
    transcript::{
        TranscriptMeasureError, TranscriptRenderError, TranscriptSlice, TranscriptViewState,
        paint_indexed_commands, prepare_slice,
    },
};

pub(super) struct PreparedPublication {
    pub(super) surfaces: Vec<Surface>,
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
    let natural = natural_height(transcript, editor, terminal_size.width, options)?;
    Ok(Size::new(
        terminal_size.width,
        u16::try_from(natural.min(usize::from(terminal_size.height)))
            .expect("live height is bounded by terminal geometry"),
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
    let prepared = prepare_slice(
        transcript,
        observed_terminal_size.width,
        appearance.transcript_config(),
    )
    .map_err(|error| PublicationPrepareError::Transcript(render_error(error)))?;
    let height = prepared.content_height();
    let mut surfaces = Vec::new();
    let page_height = usize::from(observed_terminal_size.height.max(1));
    for first in (0..height).step_by(page_height) {
        let rows =
            u16::try_from((height - first).min(page_height)).expect("publication page height");
        let mut surface = Surface::new(Size::new(prepared.width(), rows))
            .map_err(PublicationPrepareError::Allocate)?;
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), surface.size()))
            .expect("the complete publication Surface is a valid view");
        if view.clear(appearance.styles().transcript.background) == WriteOutcome::Clipped {
            return Err(PublicationPrepareError::Transcript(
                TranscriptRenderError::SurfaceConflict,
            ));
        }
        paint_indexed_commands(
            &prepared,
            &mut view,
            appearance.styles().transcript,
            &mut TranscriptViewState::at_row(first),
            &[],
        )
        .expect("publication pages match the validated index geometry");
        surfaces.push(surface);
    }
    Ok(PreparedPublication {
        surfaces,
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
