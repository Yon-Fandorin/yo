use std::{
    error::Error,
    io,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use yo_core::RuntimePoll;

use self::finalize::{LiveCleanup, LiveRunReport, finish};
use crate::{
    runner::{
        AgentConnection, DispatchOutcome, PresentationMode, RunError, RunOutcome,
        TerminationSource, TuiSession,
        session::SessionParts,
        state::{FrameError, StateEffect, StateError, TuiState},
    },
    surface::{Size, Surface},
    terminal::{
        backend::unix::{
            CrosstermEventSource, RustixTermiosDriver, TtyStateAdapter, UnixBackend,
            UnixBackendError, UnixEvent, UnixEventReader, terminal_size,
        },
        mode::{
            TerminalSession,
            fullscreen::{FullscreenRenderError, FullscreenViewport},
            inline::{InlineRenderError, InlineViewport},
            panic_route::catch_owner_panic,
            screen::{
                ScreenMode, enter_screen, render_fullscreen, render_inline, run_fullscreen_guarded,
                run_inline_guarded,
            },
        },
    },
};

mod finalize;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WORKER_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const MAX_AGENT_EVENTS_PER_TICK: usize = 256;
pub(super) type LiveBackendError = UnixBackendError<rustix::io::Errno>;

pub(super) enum LoopExit {
    UserRequested,
    TerminationRequested,
}

#[derive(Debug)]
pub(super) enum LoopError {
    Input(String),
    Agent(String),
    State(StateError),
    Frame(FrameError),
    InlineRender(InlineRenderError),
    FullscreenRender(FullscreenRenderError),
}

impl LoopError {
    pub(super) fn detail(&self) -> String {
        match self {
            Self::Input(error) => format!("reading terminal input failed: {error}"),
            Self::Agent(error) => format!("communicating with the agent failed: {error}"),
            Self::State(StateError::Transcript(error)) => {
                format!("updating transcript state failed: {error:?}")
            },
            Self::State(StateError::UnknownActivity(activity)) => {
                format!("updating unknown agent Activity failed: {activity:?}")
            },
            Self::State(StateError::ItemIdOverflow) => {
                "allocating the next transcript item ID failed".to_owned()
            },
            Self::Frame(error) => error.detail(),
            Self::InlineRender(error) => format!("rendering the inline frame failed: {error}"),
            Self::FullscreenRender(error) => {
                format!("rendering the fullscreen frame failed: {error}")
            },
        }
    }
}

impl std::fmt::Display for LoopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail())
    }
}

impl Error for LoopError {}

/// Runs one inline terminal UI session using host-owned termination state.
///
/// The process host remains responsible for signal installation, identity, and
/// final disposition. This function returns only after terminal cleanup.
pub fn run<A>(
    termination: &mut impl TerminationSource,
    agent: &mut A,
) -> Result<RunOutcome, RunError>
where
    A: AgentConnection,
{
    run_with_mode(termination, agent, PresentationMode::Inline)
}

/// Runs one terminal UI session in the explicitly selected presentation mode.
///
/// Mode selection is complete before this function acquires terminal state.
pub fn run_with_mode<A>(
    termination: &mut impl TerminationSource,
    agent: &mut A,
    mode: PresentationMode,
) -> Result<RunOutcome, RunError>
where
    A: AgentConnection,
{
    let mut session = TuiSession::new();
    run_session_with_mode(termination, agent, &mut session, mode)
}

/// Runs one terminal ownership generation for an existing TUI session.
///
/// The caller retains `session` after terminal cleanup and may pass it to a
/// later generation with the same agent connection. Each call acquires a fresh
/// presenter and frame history.
pub fn run_session_with_mode<A>(
    termination: &mut impl TerminationSource,
    agent: &mut A,
    session: &mut TuiSession,
    mode: PresentationMode,
) -> Result<RunOutcome, RunError>
where
    A: AgentConnection,
{
    finish(catch_owner_panic(AssertUnwindSafe(|| {
        run_routed(termination, agent, session, mode)
    })))
}

fn run_routed<A>(
    termination: &mut impl TerminationSource,
    agent: &mut A,
    retained: &mut TuiSession,
    mode: PresentationMode,
) -> Result<LiveRunReport, RunError>
where
    A: AgentConnection,
{
    let source = CrosstermEventSource::acquire()
        .map_err(|error| RunError::new("acquiring terminal input failed", format!("{error:?}")))?;
    let size = terminal_size()
        .map_err(|error| RunError::new("reading terminal size failed", error.to_string()))?;
    validate_size(size)?;
    let mut events = UnixEventReader::new(source, termination);

    let stdout = io::stdout();
    let output = stdout.lock();
    let tty = TtyStateAdapter::new(RustixTermiosDriver::stdin());
    let mut backend = UnixBackend::new(tty, output);
    let started = Instant::now();

    match mode {
        PresentationMode::Inline => {
            let session =
                enter_screen(&mut backend, ScreenMode::Inline).map_err(finalize::entry_failure)?;
            let mut viewport = InlineViewport::default();
            let report = run_inline_guarded(session, &mut viewport, |session, viewport| {
                drive(
                    session,
                    viewport,
                    &mut events,
                    retained,
                    agent,
                    size,
                    started,
                )
            });
            let (operation, output) = match report.operation {
                // An untrusted restore may leave the last frame visible. Replaying the current
                // chat projection can then duplicate content, which the recovery contract prefers
                // to erasing rows whose ownership is no longer provable.
                Ok(Ok(exit)) => (Ok(Ok(exit)), retained_session_output(retained.state())),
                Ok(Err(error)) => (Ok(Err(error)), None),
                Err(payload) => (Err(payload), None),
            };
            Ok(LiveRunReport {
                operation,
                cleanup: LiveCleanup::Inline(report.cleanup),
                output,
            })
        },
        PresentationMode::Fullscreen => {
            let session = enter_screen(&mut backend, ScreenMode::Fullscreen)
                .map_err(finalize::entry_failure)?;
            let mut viewport = FullscreenViewport::default();
            let report = run_fullscreen_guarded(session, |session| {
                drive(
                    session,
                    &mut viewport,
                    &mut events,
                    retained,
                    agent,
                    size,
                    started,
                )
            });
            Ok(LiveRunReport {
                operation: report.operation,
                cleanup: LiveCleanup::Fullscreen(report.cleanup),
                output: None,
            })
        },
    }
}

pub(super) fn retained_session_output(state: &TuiState) -> Option<String> {
    state.session_output().ok().flatten()
}

pub(super) fn drive<B, E, T, A, P>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    events: &mut UnixEventReader<E, T>,
    retained: &mut TuiSession,
    agent: &mut A,
    mut size: Size,
    started: Instant,
) -> Result<LoopExit, LoopError>
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
    B::Mode: PartialEq,
    E: crate::terminal::backend::unix::EventSource,
    E::Error: std::fmt::Debug,
    T: TerminationSource,
    A: AgentConnection,
    P: LivePresenter<B>,
{
    let mut previous = None;
    let mut frame_visible = false;
    let SessionParts {
        state,
        pending_dispatch,
        pending_control,
    } = retained.parts_mut();

    loop {
        if drain_agent(agent, state)? && size.width > 0 && size.height > 0 {
            redraw(session, viewport, state, size, &mut previous)?;
            frame_visible = true;
        }
        if let Some(action) = pending_control.take() {
            match agent
                .retry(action)
                .map_err(|error| LoopError::Agent(error.to_string()))?
            {
                DispatchOutcome::Accepted => {},
                DispatchOutcome::Backpressured(action) => {
                    *pending_control = Some(action);
                },
            }
        }
        if pending_control.is_none()
            && let Some(action) = pending_dispatch.take()
        {
            match agent
                .retry(action)
                .map_err(|error| LoopError::Agent(error.to_string()))?
            {
                DispatchOutcome::Accepted => {},
                DispatchOutcome::Backpressured(action) => {
                    *pending_dispatch = Some(action);
                },
            }
        }
        if pending_control.is_some() || pending_dispatch.is_some() {
            match events
                .next(WORKER_RETRY_INTERVAL)
                .map_err(|error| LoopError::Input(format!("{error:?}")))?
            {
                UnixEvent::Terminate => return Ok(LoopExit::TerminationRequested),
                UnixEvent::Input(input) => {
                    match handle_backpressured_input(
                        state,
                        input,
                        started.elapsed(),
                        pending_control.is_none(),
                    )
                    .map_err(LoopError::State)?
                    {
                        StateEffect::Exit => return Ok(LoopExit::UserRequested),
                        StateEffect::Dispatch(action) => {
                            let is_interrupt =
                                matches!(&action, crate::runner::AgentAction::Interrupt);
                            if is_interrupt {
                                *pending_dispatch = None;
                            }
                            match agent
                                .dispatch(action)
                                .map_err(|error| LoopError::Agent(error.to_string()))?
                            {
                                DispatchOutcome::Accepted => {},
                                DispatchOutcome::Backpressured(action) => {
                                    if is_interrupt {
                                        *pending_control = Some(action);
                                    } else {
                                        debug_assert!(pending_control.is_none());
                                        *pending_control = Some(action);
                                    }
                                },
                            }
                        },
                        StateEffect::Redraw => {
                            if size.width > 0 && size.height > 0 {
                                redraw(session, viewport, state, size, &mut previous)?;
                                frame_visible = true;
                            }
                        },
                        StateEffect::Unchanged => {},
                        StateEffect::Resize(next) => {
                            prepare_resize(viewport, &mut size, next);
                            frame_visible = false;
                        },
                    }
                },
                UnixEvent::Idle => {},
            }
            continue;
        }
        let timeout = if !frame_visible && size.width > 0 && size.height > 0 {
            Duration::ZERO
        } else {
            INPUT_POLL_INTERVAL
        };
        match events
            .next(timeout)
            .map_err(|error| LoopError::Input(format!("{error:?}")))?
        {
            UnixEvent::Idle => {
                if !frame_visible && size.width > 0 && size.height > 0 {
                    redraw(session, viewport, state, size, &mut previous)?;
                    frame_visible = true;
                }
            },
            UnixEvent::Terminate => return Ok(LoopExit::TerminationRequested),
            UnixEvent::Input(input) => match state
                .handle(input, started.elapsed())
                .map_err(LoopError::State)?
            {
                StateEffect::Unchanged => {},
                StateEffect::Exit => return Ok(LoopExit::UserRequested),
                StateEffect::Redraw => {
                    if size.width > 0 && size.height > 0 {
                        redraw(session, viewport, state, size, &mut previous)?;
                        frame_visible = true;
                    }
                },
                StateEffect::Dispatch(action) => {
                    match agent
                        .dispatch(action)
                        .map_err(|error| LoopError::Agent(error.to_string()))?
                    {
                        DispatchOutcome::Accepted => {},
                        DispatchOutcome::Backpressured(action) => {
                            *pending_dispatch = Some(action);
                        },
                    }
                    if size.width > 0 && size.height > 0 {
                        redraw(session, viewport, state, size, &mut previous)?;
                        frame_visible = true;
                    }
                },
                StateEffect::Resize(next) => {
                    prepare_resize(viewport, &mut size, next);
                    frame_visible = false;
                },
            },
        }
    }
}

pub(super) fn handle_backpressured_input(
    state: &mut TuiState,
    input: crate::input::event::InputEvent,
    now: Duration,
    allow_pending_request: bool,
) -> Result<StateEffect, StateError> {
    if (allow_pending_request && state.has_pending_request())
        || input.is_ctrl_c_or_d()
        || matches!(input, crate::input::event::InputEvent::Resize(_))
    {
        state.handle(input, now)
    } else {
        Ok(StateEffect::Unchanged)
    }
}

pub(super) fn drain_agent<A>(agent: &mut A, state: &mut TuiState) -> Result<bool, LoopError>
where
    A: AgentConnection,
{
    let mut changed = false;
    for _ in 0..MAX_AGENT_EVENTS_PER_TICK {
        match agent
            .poll()
            .map_err(|error| LoopError::Agent(error.to_string()))?
        {
            RuntimePoll::Pending => return Ok(changed),
            RuntimePoll::Event(event) => {
                state.observe(event).map_err(LoopError::State)?;
                changed = true;
            },
            RuntimePoll::Closed => {
                return Err(LoopError::Agent(
                    "the agent connection closed unexpectedly".to_owned(),
                ));
            },
        }
    }
    Ok(changed)
}

pub(super) fn prepare_resize<P: FrameViewport>(viewport: &mut P, size: &mut Size, next: Size) {
    viewport.invalidate_frame();
    *size = next;
}

fn redraw<B, P>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    state: &mut TuiState,
    size: Size,
    previous: &mut Option<Surface>,
) -> Result<(), LoopError>
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
    B::Mode: PartialEq,
    P: LivePresenter<B>,
{
    let frame = state.prepare_frame(size).map_err(LoopError::Frame)?;
    viewport.render(session, previous.as_ref(), &frame.surface, frame.cursor)?;
    state.commit_frame(&frame);
    *previous = Some(frame.surface);
    Ok(())
}

pub(super) trait FrameViewport {
    fn invalidate_frame(&mut self);
}

pub(super) trait LivePresenter<B>: FrameViewport
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
{
    fn render(
        &mut self,
        session: &mut TerminalSession<'_, B>,
        previous: Option<&Surface>,
        current: &Surface,
        cursor: crate::surface::Point,
    ) -> Result<(), LoopError>;
}

impl FrameViewport for InlineViewport {
    fn invalidate_frame(&mut self) {
        InlineViewport::invalidate_frame(self);
    }
}

impl<B> LivePresenter<B> for InlineViewport
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
    B::Mode: PartialEq,
{
    fn render(
        &mut self,
        session: &mut TerminalSession<'_, B>,
        previous: Option<&Surface>,
        current: &Surface,
        cursor: crate::surface::Point,
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
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
    B::Mode: PartialEq,
{
    fn render(
        &mut self,
        session: &mut TerminalSession<'_, B>,
        previous: Option<&Surface>,
        current: &Surface,
        cursor: crate::surface::Point,
    ) -> Result<(), LoopError> {
        render_fullscreen(session, self, previous, current, cursor)
            .map_err(LoopError::FullscreenRender)
    }
}

fn validate_size(size: Size) -> Result<(), RunError> {
    if size.width == 0 || size.height == 0 {
        Err(RunError::new(
            "terminal size is unavailable",
            format!("{}x{}", size.width, size.height),
        ))
    } else {
        Ok(())
    }
}
