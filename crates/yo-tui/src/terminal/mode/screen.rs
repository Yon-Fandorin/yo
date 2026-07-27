use std::{
    iter,
    panic::{AssertUnwindSafe, catch_unwind},
};

use super::{
    inline::{InlineRenderError, InlineRenderer, InlineRestoreOutcome, InlineViewport},
    panic_route::{PanicOutcome, PanicPayload, PanicRouteError, catch_owner_panic},
    transaction::{
        CleanupFailureCause, CleanupFailures, SessionFailure, TerminalSession, panic_message,
    },
};
use crate::{
    surface::Surface,
    terminal::backend::{ScreenModeBackend, TerminalBackend, TerminalOutputBackend},
};

type EntryFailure<B> = SessionFailure<
    <B as TerminalBackend>::Error,
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
        ScreenMode::Inline => TerminalSession::enter(backend, iter::empty()),
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
) -> Result<(), InlineRenderError>
where
    B: TerminalOutputBackend,
{
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(session.output());
    renderer.render(pending, previous, current)
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
    mut session: TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
    operation: impl FnOnce(&mut TerminalSession<'_, B>, &mut InlineViewport) -> T,
) -> InlineBoundaryResult<B, T>
where
    B: TerminalOutputBackend,
{
    catch_owner_panic(AssertUnwindSafe(|| {
        let operation = catch_unwind(AssertUnwindSafe(|| operation(&mut session, viewport)));
        let cleanup = close_inline(session, viewport);

        InlineRunReport { operation, cleanup }
    }))
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
