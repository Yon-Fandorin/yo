use super::{FrameViewport, LoopError};
use crate::{
    runner::{
        frame::FrameScheduler,
        state::{MotionDemand, TuiState},
    },
    surface::{Point, Size, Surface},
    terminal::{
        backend::{ScreenModeBackend, TerminalOutputBackend},
        mode::{
            TerminalSession,
            fullscreen::FullscreenViewport,
            inline::InlineViewport,
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
    ) -> Result<(), LoopError>;
}

impl FrameViewport for InlineViewport {
    fn invalidate_frame(&mut self) {
        InlineViewport::invalidate_frame(self);
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
    ) -> Result<(), LoopError> {
        render_inline(session, self, previous, current, cursor).map_err(LoopError::InlineRender)
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
    ) -> Result<(), LoopError> {
        render_fullscreen(session, self, previous, current, cursor)
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
    size: Size,
    previous: &mut Option<Surface>,
    epoch: std::time::Instant,
) -> Result<Option<std::time::Instant>, LoopError>
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
    P: LivePresenter<B>,
{
    let appearance = appearance.pin();
    let elapsed = epoch.elapsed();
    let frame = state
        .prepare_frame_at(size, &appearance, elapsed)
        .map_err(LoopError::Frame)?;
    debug_assert_eq!(frame.appearance_revision, appearance.revision());
    viewport.render(session, previous.as_ref(), &frame.surface, frame.cursor)?;
    state.commit_frame(&frame);
    let deadline = super::timing::next_motion_deadline(
        epoch,
        elapsed,
        frame.motion_demand.map(MotionDemand::period),
    );
    *previous = Some(frame.surface);
    Ok(deadline)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_requested_frame<B, P>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    state: &mut TuiState,
    appearance: &crate::appearance::AppearanceState,
    size: Size,
    previous: &mut Option<Surface>,
    epoch: std::time::Instant,
    frames: &mut FrameScheduler,
    frame_visible: &mut bool,
    motion_deadline: &mut Option<std::time::Instant>,
) -> Result<(), LoopError>
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
    P: LivePresenter<B>,
{
    let now = std::time::Instant::now();
    if size.width == 0 || size.height == 0 || !frames.is_due(now) {
        return Ok(());
    }
    *motion_deadline = redraw(session, viewport, state, appearance, size, previous, epoch)?;
    frames.rendered(std::time::Instant::now());
    *frame_visible = true;
    Ok(())
}
