use std::{
    error::Error,
    io,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use self::finalize::{LiveCleanup, LiveRunReport, finish};
use crate::{
    appearance::{ColorCapability, MotionPreference},
    runner::{
        AgentConnection, AgentPoll, DispatchOutcome, PresentationMode, RunError,
        SkillReferenceConnection, SkillReferencePoll, TerminalOutcome, TerminationSource,
        TuiSession, WorkspaceReferenceConnection, WorkspaceReferencePoll,
        session::SessionParts,
        state::{FrameError, MotionDemand, StateEffect, StateError, TuiState},
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
    User,
    Termination,
    Suspend,
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
            Self::State(StateError::SubmissionIdentityUnavailable) => {
                "allocating a submission identity failed".to_owned()
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
/// final disposition. This function returns only after terminal cleanup. A
/// suspend result closes this fresh one-shot session; callers that need reentry
/// must retain a [`TuiSession`] and use [`run_session_with_mode`].
pub fn run<A>(
    termination: &mut impl TerminationSource,
    agent: &mut A,
    color_capability: ColorCapability,
    motion_preference: MotionPreference,
) -> Result<TerminalOutcome, RunError>
where
    A: AgentConnection,
{
    run_with_mode(
        termination,
        agent,
        PresentationMode::Inline,
        color_capability,
        motion_preference,
    )
}

/// Runs one terminal UI session in the explicitly selected presentation mode.
///
/// Mode selection is complete before this function acquires terminal state. A
/// suspend result closes this fresh one-shot session; use
/// [`run_session_with_mode`] to retain application state across generations.
pub fn run_with_mode<A>(
    termination: &mut impl TerminationSource,
    agent: &mut A,
    mode: PresentationMode,
    color_capability: ColorCapability,
    motion_preference: MotionPreference,
) -> Result<TerminalOutcome, RunError>
where
    A: AgentConnection,
{
    let mut session = TuiSession::new(color_capability, motion_preference);
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
) -> Result<TerminalOutcome, RunError>
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
    retained.set_presentation_mode(mode);
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
                Ok(Ok(exit @ (LoopExit::User | LoopExit::Termination))) => {
                    (Ok(Ok(exit)), retained_session_output(retained))
                },
                Ok(Ok(LoopExit::Suspend)) => (Ok(Ok(LoopExit::Suspend)), None),
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

pub(super) fn retained_session_output(session: &TuiSession) -> Option<String> {
    session.session_output().ok().flatten()
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
    let epoch = started;
    let mut motion_deadline = None;
    let SessionParts {
        state,
        appearance,
        pending_dispatch,
        pending_control,
        workspace_references,
        skill_references,
    } = retained.parts_mut();

    loop {
        let agent_changed = drain_agent(agent, state)?;
        let workspace_changed = drain_workspace_references(workspace_references, state);
        let skill_changed = drain_skill_references(skill_references, state);
        if (agent_changed || workspace_changed || skill_changed || !frame_visible)
            && size.width > 0
            && size.height > 0
        {
            motion_deadline = redraw(
                session,
                viewport,
                state,
                appearance,
                size,
                &mut previous,
                epoch,
            )?;
            frame_visible = true;
        }
        if let Some(action) = pending_control.take() {
            match agent
                .retry(action)
                .map_err(|error| LoopError::Agent(error.to_string()))?
            {
                DispatchOutcome::Queued => {},
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
                DispatchOutcome::Queued => {},
                DispatchOutcome::Backpressured(action) => {
                    *pending_dispatch = Some(action);
                },
            }
        }
        if pending_control.is_some() || pending_dispatch.is_some() {
            match events
                .next(wait_timeout(WORKER_RETRY_INTERVAL, motion_deadline))
                .map_err(|error| LoopError::Input(format!("{error:?}")))?
            {
                UnixEvent::Terminate => return Ok(LoopExit::Termination),
                UnixEvent::Input(input) => {
                    match handle_backpressured_input(
                        state,
                        input,
                        started.elapsed(),
                        pending_control.is_none(),
                    )
                    .map_err(LoopError::State)?
                    {
                        StateEffect::Exit => return Ok(LoopExit::User),
                        StateEffect::Suspend => return Ok(LoopExit::Suspend),
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
                                DispatchOutcome::Queued => {},
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
                                motion_deadline = redraw(
                                    session,
                                    viewport,
                                    state,
                                    appearance,
                                    size,
                                    &mut previous,
                                    epoch,
                                )?;
                                frame_visible = true;
                            }
                        },
                        StateEffect::WorkspaceSearch(request) => {
                            dispatch_workspace_search(workspace_references, state, request);
                            if size.width > 0 && size.height > 0 {
                                motion_deadline = redraw(
                                    session,
                                    viewport,
                                    state,
                                    appearance,
                                    size,
                                    &mut previous,
                                    epoch,
                                )?;
                                frame_visible = true;
                            }
                        },
                        StateEffect::SkillSearch(request) => {
                            dispatch_skill_search(skill_references, state, request);
                            if size.width > 0 && size.height > 0 {
                                motion_deadline = redraw(
                                    session,
                                    viewport,
                                    state,
                                    appearance,
                                    size,
                                    &mut previous,
                                    epoch,
                                )?;
                                frame_visible = true;
                            }
                        },
                        StateEffect::Unchanged => {},
                        StateEffect::Resize(next) => {
                            prepare_resize(viewport, &mut size, next);
                            frame_visible = false;
                            motion_deadline = None;
                        },
                    }
                },
                UnixEvent::Idle => {},
            }
            if frame_visible && motion_is_due(motion_deadline) && size.width > 0 && size.height > 0
            {
                motion_deadline = redraw(
                    session,
                    viewport,
                    state,
                    appearance,
                    size,
                    &mut previous,
                    epoch,
                )?;
            }
            continue;
        }
        let timeout = wait_timeout(INPUT_POLL_INTERVAL, motion_deadline);
        match events
            .next(timeout)
            .map_err(|error| LoopError::Input(format!("{error:?}")))?
        {
            UnixEvent::Idle => {
                if (!frame_visible || motion_is_due(motion_deadline))
                    && size.width > 0
                    && size.height > 0
                {
                    motion_deadline = redraw(
                        session,
                        viewport,
                        state,
                        appearance,
                        size,
                        &mut previous,
                        epoch,
                    )?;
                    frame_visible = true;
                }
            },
            UnixEvent::Terminate => return Ok(LoopExit::Termination),
            UnixEvent::Input(input) => match state
                .handle(input, started.elapsed())
                .map_err(LoopError::State)?
            {
                StateEffect::Unchanged => {},
                StateEffect::Exit => return Ok(LoopExit::User),
                StateEffect::Suspend => return Ok(LoopExit::Suspend),
                StateEffect::Redraw => {
                    if size.width > 0 && size.height > 0 {
                        motion_deadline = redraw(
                            session,
                            viewport,
                            state,
                            appearance,
                            size,
                            &mut previous,
                            epoch,
                        )?;
                        frame_visible = true;
                    }
                },
                StateEffect::Dispatch(action) => {
                    match agent
                        .dispatch(action)
                        .map_err(|error| LoopError::Agent(error.to_string()))?
                    {
                        DispatchOutcome::Queued => {},
                        DispatchOutcome::Backpressured(action) => {
                            *pending_dispatch = Some(action);
                        },
                    }
                    if size.width > 0 && size.height > 0 {
                        motion_deadline = redraw(
                            session,
                            viewport,
                            state,
                            appearance,
                            size,
                            &mut previous,
                            epoch,
                        )?;
                        frame_visible = true;
                    }
                },
                StateEffect::WorkspaceSearch(request) => {
                    dispatch_workspace_search(workspace_references, state, request);
                    if size.width > 0 && size.height > 0 {
                        motion_deadline = redraw(
                            session,
                            viewport,
                            state,
                            appearance,
                            size,
                            &mut previous,
                            epoch,
                        )?;
                        frame_visible = true;
                    }
                },
                StateEffect::SkillSearch(request) => {
                    dispatch_skill_search(skill_references, state, request);
                    if size.width > 0 && size.height > 0 {
                        motion_deadline = redraw(
                            session,
                            viewport,
                            state,
                            appearance,
                            size,
                            &mut previous,
                            epoch,
                        )?;
                        frame_visible = true;
                    }
                },
                StateEffect::Resize(next) => {
                    prepare_resize(viewport, &mut size, next);
                    frame_visible = false;
                    motion_deadline = None;
                },
            },
        }
        if frame_visible && motion_is_due(motion_deadline) && size.width > 0 && size.height > 0 {
            motion_deadline = redraw(
                session,
                viewport,
                state,
                appearance,
                size,
                &mut previous,
                epoch,
            )?;
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
        || state.wants_global_input(&input)
        || state.wants_overlay_input(&input)
        || input.is_control_flow_key()
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
            AgentPoll::Pending => return Ok(changed),
            AgentPoll::Record(record) => {
                state.observe_record(record).map_err(LoopError::State)?;
                changed = true;
            },
            AgentPoll::Durability(durability) => {
                state
                    .observe_durability(durability)
                    .map_err(LoopError::State)?;
                changed = true;
            },
            AgentPoll::Submission(outcome) => {
                state
                    .observe_submission_outcome(outcome)
                    .map_err(LoopError::State)?;
                changed = true;
            },
            AgentPoll::Closed => {
                return Err(LoopError::Agent(
                    "the agent connection closed unexpectedly".to_owned(),
                ));
            },
        }
    }
    Ok(changed)
}

fn dispatch_workspace_search(
    connection: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    state: &mut TuiState,
    request: yo_core::WorkspaceReferenceSearchRequest,
) {
    let result = connection
        .as_deref_mut()
        .ok_or_else(|| "workspace search is unavailable".to_owned())
        .and_then(|connection| connection.search(request));
    if let Err(error) = result {
        state.observe_workspace_reference_failure(error);
    }
}

fn drain_workspace_references(
    connection: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    state: &mut TuiState,
) -> bool {
    let Some(active_connection) = connection.as_deref_mut() else {
        return false;
    };
    let mut changed = false;
    let mut disconnected = false;
    for _ in 0..MAX_AGENT_EVENTS_PER_TICK {
        match active_connection.poll() {
            Ok(WorkspaceReferencePoll::Pending) => break,
            Ok(WorkspaceReferencePoll::Update(update)) => {
                changed |= matches!(
                    state.observe_workspace_reference_update(update),
                    StateEffect::Redraw
                );
            },
            Err(error) => {
                changed |= matches!(
                    state.observe_workspace_reference_failure(error),
                    StateEffect::Redraw
                );
                disconnected = true;
                break;
            },
        }
    }
    if disconnected {
        *connection = None;
    }
    changed
}

fn dispatch_skill_search(
    connection: &mut Option<Box<dyn SkillReferenceConnection>>,
    state: &mut TuiState,
    request: yo_core::SkillReferenceSearchRequest,
) {
    let result = connection
        .as_deref_mut()
        .ok_or_else(|| "skill search is unavailable".to_owned())
        .and_then(|connection| connection.search(request));
    if let Err(error) = result {
        state.observe_skill_reference_failure(error);
    }
}

fn drain_skill_references(
    connection: &mut Option<Box<dyn SkillReferenceConnection>>,
    state: &mut TuiState,
) -> bool {
    let Some(active_connection) = connection.as_deref_mut() else {
        return false;
    };
    let mut changed = false;
    let mut disconnected = false;
    for _ in 0..MAX_AGENT_EVENTS_PER_TICK {
        match active_connection.poll() {
            Ok(SkillReferencePoll::Pending) => break,
            Ok(SkillReferencePoll::Update(update)) => {
                changed |= matches!(
                    state.observe_skill_reference_update(update),
                    StateEffect::Redraw
                );
            },
            Err(error) => {
                changed |= matches!(
                    state.observe_skill_reference_failure(error),
                    StateEffect::Redraw
                );
                disconnected = true;
                break;
            },
        }
    }
    if disconnected {
        *connection = None;
    }
    changed
}

pub(super) fn prepare_resize<P: FrameViewport>(viewport: &mut P, size: &mut Size, next: Size) {
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
    epoch: Instant,
) -> Result<Option<Instant>, LoopError>
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
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
    let deadline = next_motion_deadline(
        epoch,
        elapsed,
        frame.motion_demand.map(MotionDemand::period),
    );
    *previous = Some(frame.surface);
    Ok(deadline)
}

fn next_motion_deadline(
    epoch: Instant,
    elapsed: Duration,
    period: Option<Duration>,
) -> Option<Instant> {
    let period = period?;
    if period.is_zero() {
        return None;
    }
    let remainder = elapsed.as_nanos() % period.as_nanos();
    let remainder = Duration::new(
        u64::try_from(remainder / 1_000_000_000).ok()?,
        u32::try_from(remainder % 1_000_000_000).ok()?,
    );
    let current_tick_start = elapsed.checked_sub(remainder)?;
    epoch.checked_add(current_tick_start.checked_add(period)?)
}

fn wait_timeout(base: Duration, deadline: Option<Instant>) -> Duration {
    let Some(deadline) = deadline else {
        return base;
    };
    base.min(deadline.saturating_duration_since(Instant::now()))
}

fn motion_is_due(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
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

#[cfg(test)]
mod motion_tests {
    use std::time::{Duration, Instant};

    use super::next_motion_deadline;

    // 늦게 깨어난 frame은 놓친 tick을 재생하지 않고 epoch 기준 다음 경계 하나만 예약한다.
    #[test]
    fn late_frame_skips_missed_ticks_and_targets_the_next_epoch_boundary() {
        let epoch = Instant::now();
        let deadline = next_motion_deadline(
            epoch,
            Duration::from_millis(370),
            Some(Duration::from_millis(120)),
        )
        .unwrap();

        assert_eq!(deadline.duration_since(epoch), Duration::from_millis(480));
    }

    // 정확한 tick 경계에서 그린 frame도 같은 경계를 다시 요구하지 않고 다음 tick을 예약한다.
    #[test]
    fn exact_tick_boundary_schedules_the_following_tick() {
        let epoch = Instant::now();
        let deadline = next_motion_deadline(
            epoch,
            Duration::from_millis(120),
            Some(Duration::from_millis(120)),
        )
        .unwrap();

        assert_eq!(deadline.duration_since(epoch), Duration::from_millis(240));
    }

    // frame이 motion을 요구하지 않으면 runner는 별도의 시간 기반 wakeup을 만들지 않는다.
    #[test]
    fn absent_motion_demand_disarms_the_deadline() {
        assert_eq!(
            next_motion_deadline(Instant::now(), Duration::from_secs(1), None),
            None
        );
    }
}
