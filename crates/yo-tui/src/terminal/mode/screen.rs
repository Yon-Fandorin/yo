use std::{
    iter,
    panic::{AssertUnwindSafe, catch_unwind},
};

use super::{
    fullscreen::{FullscreenRenderError, FullscreenRenderer, FullscreenViewport},
    inline::{InlineRenderError, InlineRenderer, InlineRestoreOutcome, InlineViewport},
    panic_route::{PanicOutcome, PanicPayload, PanicRouteError, catch_owner_panic},
    transaction::{
        CleanupFailureCause, CleanupFailures, SessionFailure, SessionFailureCause, TerminalSession,
        panic_message,
    },
};
use crate::{
    surface::{Point, Surface},
    terminal::backend::{ScreenModeBackend, TerminalBackend, TerminalOutputBackend},
};

type EntryFailure<B> = SessionFailure<
    SessionFailureCause<<B as TerminalBackend>::Error>,
    <B as TerminalBackend>::Mode,
    <B as TerminalBackend>::Error,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScreenMode {
    Inline,
    Fullscreen,
}

pub(crate) fn enter_screen<B>(
    backend: &mut B,
    screen_mode: ScreenMode,
) -> Result<TerminalSession<'_, B>, EntryFailure<B>>
where
    B: ScreenModeBackend,
{
    match screen_mode {
        ScreenMode::Inline => {
            TerminalSession::enter(backend, iter::once(B::cursor_visibility_mode()))
        },
        ScreenMode::Fullscreen => {
            TerminalSession::enter(backend, iter::once(B::alternate_screen_mode()))
        },
    }
}

pub(crate) fn render_inline<B>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
    previous: Option<&Surface>,
    current: &Surface,
    cursor: Point,
) -> Result<(), InlineRenderError>
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
{
    if session.owns_mode(B::alternate_screen_mode()) {
        return Err(InlineRenderError::AlternateScreenOwned);
    }
    if !session.owns_mode(B::cursor_visibility_mode()) {
        return Err(InlineRenderError::CursorVisibilityNotOwned);
    }
    let pending = viewport
        .begin_frame_at(current.size(), cursor)
        .map_err(InlineRenderError::Frame)?;
    let mut renderer = InlineRenderer::new(session.output());
    renderer.render(pending, previous, current)
}

pub(crate) fn render_fullscreen<B>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut FullscreenViewport,
    previous: Option<&Surface>,
    current: &Surface,
    cursor: Point,
) -> Result<(), FullscreenRenderError>
where
    B: ScreenModeBackend + TerminalOutputBackend,
    B::Mode: PartialEq,
{
    if !session.owns_mode(B::alternate_screen_mode()) {
        return Err(FullscreenRenderError::AlternateScreenNotOwned);
    }
    let pending = viewport
        .begin_frame(current.size(), cursor)
        .map_err(FullscreenRenderError::Frame)?;
    FullscreenRenderer::new(session.output()).render(pending, previous, current)
}

#[derive(Debug)]
pub(crate) struct InlineCloseReport<M, E> {
    pub(crate) viewport: Result<InlineRestoreOutcome, CleanupFailureCause<InlineRenderError>>,
    pub(crate) terminal: Result<(), CleanupFailures<M, E>>,
}

pub(crate) struct InlineRunReport<T, M, E> {
    pub(crate) operation: Result<T, PanicPayload>,
    pub(crate) cleanup: InlineCloseReport<M, E>,
}

pub(crate) struct FullscreenRunReport<T, M, E> {
    pub(crate) operation: Result<T, PanicPayload>,
    pub(crate) cleanup: Result<(), CleanupFailures<M, E>>,
}

type InlineBoundaryResult<B, T> = Result<
    PanicOutcome<InlineRunReport<T, <B as TerminalBackend>::Mode, <B as TerminalBackend>::Error>>,
    PanicRouteError,
>;

type FullscreenBoundaryResult<B, T> = Result<
    PanicOutcome<
        FullscreenRunReport<T, <B as TerminalBackend>::Mode, <B as TerminalBackend>::Error>,
    >,
    PanicRouteError,
>;

pub(crate) fn run_inline_boundary<B, T>(
    session: TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
    operation: impl FnOnce(&mut TerminalSession<'_, B>, &mut InlineViewport) -> T,
) -> InlineBoundaryResult<B, T>
where
    B: TerminalOutputBackend,
{
    catch_owner_panic(AssertUnwindSafe(|| {
        run_inline_guarded(session, viewport, operation)
    }))
}

pub(crate) fn run_inline_guarded<B, T>(
    mut session: TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
    operation: impl FnOnce(&mut TerminalSession<'_, B>, &mut InlineViewport) -> T,
) -> InlineRunReport<T, B::Mode, B::Error>
where
    B: TerminalOutputBackend,
{
    let operation = catch_unwind(AssertUnwindSafe(|| operation(&mut session, viewport)));
    let cleanup = close_inline(session, viewport);

    InlineRunReport { operation, cleanup }
}

pub(crate) fn run_fullscreen_boundary<B, T>(
    mut session: TerminalSession<'_, B>,
    operation: impl FnOnce(&mut TerminalSession<'_, B>) -> T,
) -> FullscreenBoundaryResult<B, T>
where
    B: TerminalBackend,
{
    catch_owner_panic(AssertUnwindSafe(|| {
        let operation = catch_unwind(AssertUnwindSafe(|| operation(&mut session)));
        let cleanup = session.close();

        FullscreenRunReport { operation, cleanup }
    }))
}

pub(crate) fn close_inline<B>(
    mut session: TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
) -> InlineCloseReport<B::Mode, B::Error>
where
    B: TerminalOutputBackend,
{
    let viewport = match catch_unwind(AssertUnwindSafe(|| {
        let pending = viewport.begin_restore();
        InlineRenderer::new(session.output()).restore(pending)
    })) {
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(error)) => Err(CleanupFailureCause::Error(error)),
        Err(payload) => Err(CleanupFailureCause::Panicked(panic_message(payload))),
    };
    let terminal = session.close();

    InlineCloseReport { viewport, terminal }
}

#[cfg(test)]
mod tests;
