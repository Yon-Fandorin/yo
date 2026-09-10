//! Frame preparation and presentation snapshots for the live TUI state.

use std::time::Duration;

use yo_core::JournalDurability;

use super::{PendingRequest, TuiState};
use crate::{
    appearance::{AppearancePin, AppearanceRevision},
    overlay::OverlayPresentation,
    runner::{
        PresentationMode,
        publication::{self, PreparedPublication, PublicationPrepareError},
        view::{ObservabilityRenderError, ObservabilityRenderOptions, ObservabilityViewState},
    },
    shell::{AgentShellMeasureError, AgentShellRenderOptions, RequestPrompt, ShellChromeSnapshot},
    surface::{Point, Rect, Size, Surface, SurfaceError},
};

#[derive(Debug)]
pub(in crate::runner) enum FrameError {
    Allocate(SurfaceError),
    Measure(AgentShellMeasureError),
    Publication(PublicationPrepareError),
    Render(ObservabilityRenderError),
}

impl FrameError {
    pub(in crate::runner) fn detail(&self) -> String {
        match self {
            Self::Allocate(error) => format!("allocating the frame failed: {error}"),
            Self::Measure(error) => format!("measuring the compact agent shell failed: {error:?}"),
            Self::Publication(error) => error.detail(),
            Self::Render(error) => format!("composing the agent shell failed: {error:?}"),
        }
    }
}

pub(in crate::runner) struct PreparedFrame {
    pub(in crate::runner) surface: Surface,
    pub(in crate::runner) publication: Option<PreparedPublication>,
    pub(in crate::runner) cursor: Point,
    pub(in crate::runner) appearance_revision: AppearanceRevision,
    pub(in crate::runner) motion_demand: Option<MotionDemand>,
    pub(in crate::runner) overlay_presented: bool,
    pub(in crate::runner) overlay_presentation: Option<OverlayPresentation>,
    pub(in crate::runner) reprepare_for_publication: bool,
    pub(super) view_state: ObservabilityViewState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runner) struct MotionDemand {
    period: Duration,
    marker_period: Option<Duration>,
}

impl TuiState {
    #[cfg(test)]
    pub(in crate::runner) fn prepare_frame(
        &self,
        size: Size,
        appearance: &AppearancePin,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at(size, appearance, Duration::ZERO)
    }

    #[cfg(test)]
    pub(in crate::runner) fn prepare_frame_at(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(size, appearance, elapsed, 0, false, || {})
    }

    pub(in crate::runner) fn prepare_frame_for_geometry(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
        geometry_epoch: u64,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(
            size,
            appearance,
            elapsed,
            geometry_epoch,
            true,
            || {},
        )
    }

    #[cfg(test)]
    pub(in crate::runner) fn prepare_frame_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
        after_measure: impl FnOnce(),
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(
            size,
            appearance,
            Duration::ZERO,
            0,
            false,
            after_measure,
        )
    }

    fn prepare_frame_at_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
        geometry_epoch: u64,
        publication_enabled: bool,
        after_measure: impl FnOnce(),
    ) -> Result<PreparedFrame, FrameError> {
        let snapshot = appearance.snapshot();
        if let Some(preview) = self.preview.as_ref() {
            return preview.state.prepare_frame_at_with_measure_hook(
                size,
                appearance,
                elapsed,
                geometry_epoch,
                false,
                after_measure,
            );
        }
        let publication_eligible = publication_enabled
            && self.presentation_mode == PresentationMode::Inline
            && self.views.inline_publication_eligible();
        let candidate = publication_eligible
            .then(|| self.chat.publication_candidate())
            .flatten();
        let live_start = if publication_eligible {
            candidate.as_ref().map_or_else(
                || self.chat.published_item_count(),
                |candidate| candidate.range().end,
            )
        } else {
            0
        };
        let live_transcript = self.chat.transcript().suffix(live_start);
        let overlay_presentation = self.overlay.presentation();
        let render_options = ObservabilityRenderOptions {
            appearance: snapshot,
            chrome: self.chrome_snapshot(),
            elapsed,
            overlay: self.overlay.panel(),
            overlay_bindings: self.overlay.bindings(),
        };
        let frame_size = if publication_eligible {
            let transcript_config = self.views.chat_transcript_config(snapshot);
            let shell_options = AgentShellRenderOptions {
                transcript_config: &transcript_config,
                styles: snapshot.styles(),
                scroll: &[],
                frame_prompt: true,
                chrome: render_options.chrome,
                activity_motion: snapshot.activity_motion_frame(elapsed),
                overlay: render_options.overlay,
                overlay_bindings: render_options.overlay_bindings,
            };
            publication::compact_live_size(live_transcript, &self.editor, size, shell_options)
                .map_err(FrameError::Measure)?
        } else {
            size
        };
        let publication = candidate
            .map(|candidate| {
                publication::prepare(
                    self.chat.transcript().slice(candidate.range()),
                    candidate,
                    size,
                    geometry_epoch,
                    appearance.revision(),
                    snapshot,
                )
            })
            .transpose()
            .map_err(FrameError::Publication)?;
        let mut surface = Surface::new(frame_size).map_err(FrameError::Allocate)?;
        let frame = {
            let area = Rect::new(Point::new(0, 0), frame_size);
            let mut view = surface
                .view(area)
                .expect("the complete surface is always a valid view");
            self.views
                .render(
                    live_transcript,
                    &self.editor,
                    &mut view,
                    render_options,
                    after_measure,
                )
                .map_err(FrameError::Render)?
        };

        Ok(PreparedFrame {
            surface,
            publication,
            cursor: frame.cursor,
            appearance_revision: appearance.revision(),
            motion_demand: frame.motion_period.map(|period| MotionDemand {
                period,
                marker_period: snapshot.activity_motion_frame(elapsed).marker_interval(),
            }),
            view_state: frame.state,
            overlay_presented: frame.overlay_presented,
            overlay_presentation,
            reprepare_for_publication: publication_enabled
                && self.presentation_mode == PresentationMode::Inline
                && !publication_eligible
                && frame.state.inline_publication_eligible(),
        })
    }

    fn chrome_snapshot(&self) -> ShellChromeSnapshot<'_> {
        ShellChromeSnapshot {
            image_thumbnail: self.prompt_assist.image_thumbnail(),
            turn_active: self.active_turn.is_some(),
            queued_messages: self.follow_ups.len(),
            queue_paused: self.follow_ups_paused,
            request: self.pending_requests.front().map(|request| match request {
                PendingRequest::Approval(_) => RequestPrompt::Approval,
                PendingRequest::UserInput(_) if self.question_notes.is_some() => {
                    RequestPrompt::Notes
                },
                PendingRequest::UserInput(request)
                    if self
                        .chat
                        .question(request.activity())
                        .is_some_and(|question| {
                            question.allow_notes && !question.choices.is_empty()
                        }) =>
                {
                    RequestPrompt::Choice
                },
                PendingRequest::UserInput(_) => RequestPrompt::Answer,
            }),
            backend: self.session_info.backend(),
            usage: self.chat.latest_usage(),
            status: (!self.host_status.as_str().is_empty()).then_some(self.host_status.as_str()),
            storage_warning: matches!(self.durability, Some(JournalDurability::Gap { .. }))
                .then_some("History not saved"),
            workspace: self.session_info.workspace(),
            mode: self.presentation_mode,
        }
    }
}

impl MotionDemand {
    pub(in crate::runner) const fn marker_period(self) -> Option<Duration> {
        self.marker_period
    }

    pub(in crate::runner) const fn period(self) -> Duration {
        self.period
    }
}
