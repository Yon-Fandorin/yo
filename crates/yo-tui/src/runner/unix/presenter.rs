use super::{FrameViewport, LoopError};
use crate::{
    runner::{
        frame::FrameScheduler,
        session::PublicationRecoveryEvidence,
        state::{MotionDemand, TuiState},
    },
    surface::{Point, Size, Surface},
    terminal::{
        backend::{ScreenModeBackend, TerminalOutputBackend},
        mode::{
            TerminalSession,
            fullscreen::FullscreenViewport,
            inline::{InlineRecovery, InlineViewport},
            screen::{render_fullscreen, render_inline},
        },
    },
};

pub(in crate::runner) trait LivePresenter<B>: FrameViewport
where
    B: ScreenModeBackend + TerminalOutputBackend,
{
    fn render(
        &mut self,
        session: &mut TerminalSession<'_, B>,
        previous: Option<&Surface>,
        current: &Surface,
        cursor: Point,
        publication: Option<&Surface>,
        terminal_size: Size,
    ) -> Result<RenderReceipt, LoopError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::runner) struct RenderReceipt {
    pub(in crate::runner) publication_complete: bool,
    pub(in crate::runner) publication_recovery: Option<InlineRecovery>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GeometryObservation {
    pub(super) resize_count: u64,
    pub(super) sampled_size: Size,
}

pub(super) struct PresentationState<'session> {
    pub(super) size: Size,
    pub(super) geometry_epoch: u64,
    pub(super) previous: Option<Surface>,
    pub(super) started: std::time::Instant,
    pub(super) frame_visible: bool,
    pub(super) motion_deadline: Option<std::time::Instant>,
    recovery_evidence: &'session mut PublicationRecoveryEvidence,
}

impl<'session> PresentationState<'session> {
    pub(super) fn new(
        size: Size,
        started: std::time::Instant,
        recovery_evidence: &'session mut PublicationRecoveryEvidence,
    ) -> Self {
        Self {
            size,
            geometry_epoch: 0,
            previous: None,
            started,
            frame_visible: false,
            motion_deadline: None,
            recovery_evidence,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RedrawOutcome {
    motion_deadline: Option<std::time::Instant>,
    live_committed: bool,
    request_immediate: bool,
}

impl FrameViewport for InlineViewport {
    fn invalidate_frame(&mut self) {
        InlineViewport::invalidate_frame(self);
    }

    fn abandon_frame(&mut self) {
        InlineViewport::abandon_anchor(self);
    }
}

impl<B> LivePresenter<B> for InlineViewport
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
{
    fn render(
        &mut self,
        session: &mut TerminalSession<'_, B>,
        previous: Option<&Surface>,
        current: &Surface,
        cursor: Point,
        publication: Option<&Surface>,
        terminal_size: Size,
    ) -> Result<RenderReceipt, LoopError> {
        render_inline(
            session,
            self,
            previous,
            current,
            cursor,
            publication,
            terminal_size,
        )
        .map(|receipt| RenderReceipt {
            publication_complete: receipt.publication_complete,
            publication_recovery: receipt.recovery,
        })
        .map_err(LoopError::InlineRender)
    }
}

impl FrameViewport for FullscreenViewport {
    fn invalidate_frame(&mut self) {
        FullscreenViewport::invalidate_frame(self);
    }
}

impl<B> LivePresenter<B> for FullscreenViewport
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
{
    fn render(
        &mut self,
        session: &mut TerminalSession<'_, B>,
        previous: Option<&Surface>,
        current: &Surface,
        cursor: Point,
        _publication: Option<&Surface>,
        _terminal_size: Size,
    ) -> Result<RenderReceipt, LoopError> {
        render_fullscreen(session, self, previous, current, cursor)
            .map(|()| RenderReceipt::default())
            .map_err(LoopError::FullscreenRender)
    }
}

pub(in crate::runner) fn prepare_resize<P: FrameViewport>(
    viewport: &mut P,
    size: &mut Size,
    next: Size,
) {
    viewport.invalidate_frame();
    *size = next;
}

fn redraw<B, P>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    state: &mut TuiState,
    appearance: &crate::appearance::AppearanceState,
    presentation: &mut PresentationState<'_>,
    observe_geometry: &mut impl FnMut() -> Result<GeometryObservation, LoopError>,
) -> Result<RedrawOutcome, LoopError>
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
    P: LivePresenter<B>,
{
    let appearance = appearance.pin();
    let elapsed = presentation.started.elapsed();
    let frame = state
        .prepare_frame_for_geometry(
            presentation.size,
            &appearance,
            elapsed,
            presentation.geometry_epoch,
        )
        .map_err(LoopError::Frame)?;
    debug_assert_eq!(frame.appearance_revision, appearance.revision());
    let receipt = viewport.render(
        session,
        presentation.previous.as_ref(),
        &frame.surface,
        frame.cursor,
        frame
            .publication
            .as_ref()
            .map(|publication| &publication.surface),
        presentation.size,
    )?;
    if let Some(recovery) = receipt.publication_recovery {
        presentation.recovery_evidence.record(recovery);
    }
    let deadline = super::timing::next_motion_deadline(
        presentation.started,
        elapsed,
        frame.motion_demand.map(MotionDemand::period),
    );
    let Some(publication) = frame.publication.as_ref() else {
        state.commit_frame(&frame);
        presentation.previous = Some(frame.surface);
        return Ok(RedrawOutcome {
            motion_deadline: deadline,
            live_committed: true,
            request_immediate: frame.reprepare_for_publication,
        });
    };
    if !receipt.publication_complete {
        viewport.abandon_frame();
        presentation.previous = None;
        return Err(LoopError::State(
            crate::runner::state::StateError::StalePublication,
        ));
    }
    debug_assert_eq!(publication.appearance_revision, frame.appearance_revision);
    let observation = match observe_geometry() {
        Ok(observation) => observation,
        Err(error) => {
            acknowledge_publication(state, &frame)?;
            viewport.abandon_frame();
            presentation.previous = None;
            return Err(error);
        },
    };
    let prepared_epoch = publication.geometry_epoch;
    let prepared_size = publication.observed_terminal_size;
    let sample_changed_without_notification =
        observation.resize_count == 0 && observation.sampled_size != prepared_size;
    let advances = observation
        .resize_count
        .checked_add(u64::from(sample_changed_without_notification));
    let next_epoch =
        advances.and_then(|advances| presentation.geometry_epoch.checked_add(advances));
    let Some(next_epoch) = next_epoch else {
        acknowledge_publication(state, &frame)?;
        viewport.abandon_frame();
        presentation.previous = None;
        return Err(LoopError::GeometryEpochOverflow);
    };
    presentation.geometry_epoch = next_epoch;
    presentation.size = observation.sampled_size;
    acknowledge_publication(state, &frame)?;

    let live_current = observation.resize_count == 0
        && observation.sampled_size == prepared_size
        && presentation.geometry_epoch == prepared_epoch;
    if live_current {
        state.commit_frame(&frame);
        presentation.previous = Some(frame.surface);
    } else {
        viewport.abandon_frame();
        presentation.previous = None;
    }
    Ok(RedrawOutcome {
        motion_deadline: live_current.then_some(deadline).flatten(),
        live_committed: live_current,
        request_immediate: !live_current,
    })
}

fn acknowledge_publication(
    state: &mut TuiState,
    frame: &crate::runner::state::PreparedFrame,
) -> Result<(), LoopError> {
    if state.acknowledge_publication(frame) {
        Ok(())
    } else {
        Err(LoopError::State(
            crate::runner::state::StateError::StalePublication,
        ))
    }
}

pub(super) fn render_requested_frame<B, P>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    state: &mut TuiState,
    appearance: &crate::appearance::AppearanceState,
    presentation: &mut PresentationState<'_>,
    frames: &mut FrameScheduler,
    observe_geometry: &mut impl FnMut() -> Result<GeometryObservation, LoopError>,
) -> Result<(), LoopError>
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
    P: LivePresenter<B>,
{
    let now = std::time::Instant::now();
    if presentation.size.width == 0 || presentation.size.height == 0 {
        frames.suppress_pending();
        return Ok(());
    }
    if !frames.is_due(now) {
        return Ok(());
    }
    let outcome = redraw(
        session,
        viewport,
        state,
        appearance,
        presentation,
        observe_geometry,
    )?;
    presentation.motion_deadline = outcome.motion_deadline;
    frames.rendered(std::time::Instant::now());
    presentation.frame_visible = outcome.live_committed;
    if outcome.request_immediate {
        frames.request(crate::runner::frame::FrameRequest::Immediate);
    }
    Ok(())
}
