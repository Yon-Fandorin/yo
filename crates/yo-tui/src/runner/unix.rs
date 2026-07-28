use std::{
    error::Error,
    io,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use self::finalize::finish;
use crate::{
    runner::{
        RunError, RunOutcome, TerminationSource,
        state::{FrameError, StateEffect, StateError, TuiState},
    },
    surface::{Size, Surface},
    terminal::{
        backend::unix::{
            CrosstermEventSource, RustixTermiosDriver, TtyStateAdapter, UnixBackend,
            UnixBackendError, UnixEvent, UnixEventReader, UnixMode, terminal_size,
        },
        mode::{
            TerminalSession,
            inline::{InlineRenderError, InlineViewport},
            panic_route::catch_owner_panic,
            screen::{
                InlineRunReport, ScreenMode, enter_screen, render_inline, run_inline_guarded,
            },
        },
    },
};

mod finalize;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);
pub(super) type LiveBackendError = UnixBackendError<rustix::io::Errno>;
pub(super) type LiveInlineReport =
    InlineRunReport<Result<LoopExit, LoopError>, UnixMode, LiveBackendError>;

pub(super) enum LoopExit {
    UserRequested,
    TerminationRequested,
}

#[derive(Debug)]
pub(super) enum LoopError {
    Input(String),
    State(StateError),
    Frame(FrameError),
    Render(InlineRenderError),
}

impl LoopError {
    pub(super) fn detail(&self) -> String {
        match self {
            Self::Input(error) => format!("reading terminal input failed: {error}"),
            Self::State(StateError::Transcript(error)) => {
                format!("updating transcript state failed: {error:?}")
            },
            Self::State(StateError::ItemIdOverflow) => {
                "allocating the next transcript item ID failed".to_owned()
            },
            Self::Frame(error) => error.detail(),
            Self::Render(error) => format!("rendering the inline frame failed: {error}"),
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
pub fn run(termination: &mut impl TerminationSource) -> Result<RunOutcome, RunError> {
    finish(catch_owner_panic(AssertUnwindSafe(|| {
        run_routed(termination)
    })))
}

fn run_routed(termination: &mut impl TerminationSource) -> Result<LiveInlineReport, RunError> {
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
    let session =
        enter_screen(&mut backend, ScreenMode::Inline).map_err(finalize::entry_failure)?;
    let mut viewport = InlineViewport::default();
    let mut state = TuiState::new();
    let started = Instant::now();

    Ok(run_inline_guarded(
        session,
        &mut viewport,
        |session, viewport| drive(session, viewport, &mut events, &mut state, size, started),
    ))
}

fn drive<B, E, T>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
    events: &mut UnixEventReader<E, T>,
    state: &mut TuiState,
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
{
    let mut previous = None;
    let mut frame_visible = false;

    loop {
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
                StateEffect::Resize(next) => {
                    prepare_resize(viewport, &mut size, next);
                    frame_visible = false;
                },
            },
        }
    }
}

pub(super) fn prepare_resize(viewport: &mut InlineViewport, size: &mut Size, next: Size) {
    viewport.invalidate_frame();
    *size = next;
}

fn redraw<B>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut InlineViewport,
    state: &mut TuiState,
    size: Size,
    previous: &mut Option<Surface>,
) -> Result<(), LoopError>
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
    B::Mode: PartialEq,
{
    let frame = state.prepare_frame(size).map_err(LoopError::Frame)?;
    render_inline(
        session,
        viewport,
        previous.as_ref(),
        &frame.surface,
        frame.cursor,
    )
    .map_err(LoopError::Render)?;
    state.commit_frame(&frame);
    *previous = Some(frame.surface);
    Ok(())
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
