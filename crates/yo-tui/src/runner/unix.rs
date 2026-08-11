use std::{
    error::Error,
    io,
    panic::AssertUnwindSafe,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};

use self::finalize::{LiveCleanup, LiveRunReport, finish};
use crate::{
    appearance::{ColorCapability, MotionPreference},
    runner::{
        AgentConnection, AgentPoll, DispatchOutcome, PresentationMode, RunError,
        SkillReferenceConnection, SkillReferencePoll, TerminalOutcome, TerminationSource,
        TuiSession, WorkspaceReferenceConnection, WorkspaceReferencePoll,
        frame::{FrameRequest, FrameScheduler},
        session::SessionParts,
        source_schedule::{OrdinarySource, SourceSchedule},
        state::{FrameError, MotionDemand, StateEffect, StateError, TuiState},
    },
    surface::{Size, Surface},
    terminal::{
        backend::unix::{
            CrosstermEventSource, RustixTermiosDriver, TtyStateAdapter, UnixBackend,
            UnixBackendError, UnixEventReader, terminal_size,
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

const WORKER_RETRY_INTERVAL: Duration = Duration::from_millis(10);
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
    let outcome = finish(catch_owner_panic(AssertUnwindSafe(|| {
        run_routed(termination, agent, session, mode)
    })))?;
    if matches!(
        outcome,
        TerminalOutcome::Exited(crate::runner::RunOutcome {
            reason: crate::runner::ExitReason::UserRequested,
            ..
        })
    ) && let Some(selection) = session.take_model_selection()
    {
        return Ok(TerminalOutcome::ModelSelectionRequested(selection));
    }
    Ok(outcome)
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
    let waker = Waker::from(Arc::new(OwnerThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let SessionParts {
        state,
        appearance,
        pending_dispatch,
        pending_control,
        frame_rate_limit,
        workspace_references,
        skill_references,
    } = retained.parts_mut();
    let mut frames = FrameScheduler::new(frame_rate_limit);
    let mut source_schedule = SourceSchedule::default();
    frames.request(FrameRequest::Immediate);

    loop {
        request_due_motion(
            &mut frames,
            frame_visible,
            &mut motion_deadline,
            Instant::now(),
        );
        render_requested_frame(
            session,
            viewport,
            state,
            appearance,
            size,
            &mut previous,
            epoch,
            &mut frames,
            &mut frame_visible,
            &mut motion_deadline,
        )?;

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
        let backpressured = pending_control.is_some() || pending_dispatch.is_some();
        let base = backpressured.then_some(WORKER_RETRY_INTERVAL);
        let timeout = wait_timeout(base, motion_deadline, frames.deadline(Instant::now()));
        let observation = match poll_ordinary(
            events,
            agent,
            workspace_references,
            skill_references,
            &source_schedule,
            &mut context,
        )? {
            OrdinaryPoll::Pending => {
                events.wait(timeout);
                continue;
            },
            OrdinaryPoll::Reselect => continue,
            OrdinaryPoll::Termination => return Ok(LoopExit::Termination),
            OrdinaryPoll::Ready {
                source,
                observation,
            } => {
                source_schedule.handled(source);
                observation
            },
        };
        match observation {
            OrdinaryObservation::Agent(observation) => {
                if apply_agent_poll(state, observation)? {
                    frames.request(FrameRequest::Coalesced);
                }
            },
            OrdinaryObservation::Workspace(observation) => {
                let changed = apply_workspace_poll(state, workspace_references, observation);
                if changed {
                    frames.request(FrameRequest::Coalesced);
                }
            },
            OrdinaryObservation::Skill(observation) => {
                let changed = apply_skill_poll(state, skill_references, observation);
                if changed {
                    frames.request(FrameRequest::Coalesced);
                }
            },
            OrdinaryObservation::Input(input) => {
                let effect = if backpressured {
                    handle_backpressured_input(
                        state,
                        input,
                        started.elapsed(),
                        pending_control.is_none(),
                    )
                } else {
                    state.handle(input, started.elapsed())
                }
                .map_err(LoopError::State)?;
                match effect {
                    StateEffect::Unchanged => {},
                    StateEffect::Exit => return Ok(LoopExit::User),
                    StateEffect::Suspend => return Ok(LoopExit::Suspend),
                    StateEffect::Redraw => {
                        frames.request(FrameRequest::Coalesced);
                    },
                    StateEffect::Dispatch(action) => {
                        let is_interrupt = matches!(&action, crate::runner::AgentAction::Interrupt);
                        if is_interrupt {
                            *pending_dispatch = None;
                        }
                        match agent
                            .dispatch(action)
                            .map_err(|error| LoopError::Agent(error.to_string()))?
                        {
                            DispatchOutcome::Queued => {},
                            DispatchOutcome::Backpressured(action) => {
                                if backpressured || is_interrupt {
                                    *pending_control = Some(action);
                                } else {
                                    *pending_dispatch = Some(action);
                                }
                            },
                        }
                        frames.request(FrameRequest::Coalesced);
                    },
                    StateEffect::WorkspaceSearch(request) => {
                        dispatch_workspace_search(workspace_references, state, request);
                        frames.request(FrameRequest::Coalesced);
                    },
                    StateEffect::SkillSearch(request) => {
                        dispatch_skill_search(skill_references, state, request);
                        frames.request(FrameRequest::Coalesced);
                    },
                    StateEffect::Resize(next) => {
                        prepare_resize(viewport, &mut size, next);
                        frame_visible = false;
                        motion_deadline = None;
                        frames.request(FrameRequest::Immediate);
                    },
                }
            },
        }
    }
}

pub(super) enum OrdinaryObservation {
    Input(crate::input::event::InputEvent),
    Agent(AgentPoll),
    Workspace(Result<WorkspaceReferencePoll, String>),
    Skill(Result<SkillReferencePoll, String>),
}

enum ReferenceSourcePoll<P> {
    Pending,
    Reselect,
    Termination,
    Ready(Result<P, String>),
}

// Keeping one bounded observation inline avoids a heap allocation in the owner-thread hot path.
#[allow(clippy::large_enum_variant)]
pub(super) enum OrdinaryPoll {
    Pending,
    Reselect,
    Termination,
    Ready {
        source: OrdinarySource,
        observation: OrdinaryObservation,
    },
}

pub(super) fn poll_ordinary<E, T, A>(
    events: &mut UnixEventReader<E, T>,
    agent: &mut A,
    workspace_references: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    skill_references: &mut Option<Box<dyn SkillReferenceConnection>>,
    schedule: &SourceSchedule,
    context: &mut Context<'_>,
) -> Result<OrdinaryPoll, LoopError>
where
    E: crate::terminal::backend::unix::EventSource,
    E::Error: std::fmt::Debug,
    T: TerminationSource,
    A: AgentConnection,
{
    if events.poll_termination(context).is_ready() {
        return Ok(OrdinaryPoll::Termination);
    }

    let mut must_reselect = false;
    for source in schedule.order() {
        let observation = match source {
            OrdinarySource::Terminal => {
                let polled = events.poll_input(context);
                if events.poll_termination(context).is_ready() {
                    return Ok(OrdinaryPoll::Termination);
                }
                match polled {
                    Poll::Ready(result) => Some(OrdinaryObservation::Input(
                        result.map_err(|error| LoopError::Input(format!("{error:?}")))?,
                    )),
                    Poll::Pending => None,
                }
            },
            OrdinarySource::Agent => {
                let ready = agent.poll_ready(context);
                if events.poll_termination(context).is_ready() {
                    return Ok(OrdinaryPoll::Termination);
                }
                if ready.is_pending() {
                    None
                } else {
                    let polled = agent
                        .poll()
                        .map_err(|error| LoopError::Agent(error.to_string()));
                    if events.poll_termination(context).is_ready() {
                        return Ok(OrdinaryPoll::Termination);
                    }
                    match polled? {
                        AgentPoll::Pending => {
                            must_reselect |= agent.poll_ready(context).is_ready();
                            if events.poll_termination(context).is_ready() {
                                return Ok(OrdinaryPoll::Termination);
                            }
                            None
                        },
                        observation => Some(OrdinaryObservation::Agent(observation)),
                    }
                }
            },
            OrdinarySource::Workspace => {
                let Some(connection) = workspace_references.as_mut() else {
                    continue;
                };
                match poll_reference_source(
                    events,
                    connection,
                    context,
                    |connection, context| connection.poll_ready(context),
                    |connection| connection.poll(),
                    |observation| matches!(observation, WorkspaceReferencePoll::Pending),
                ) {
                    ReferenceSourcePoll::Pending => None,
                    ReferenceSourcePoll::Reselect => {
                        must_reselect = true;
                        None
                    },
                    ReferenceSourcePoll::Termination => return Ok(OrdinaryPoll::Termination),
                    ReferenceSourcePoll::Ready(observation) => {
                        Some(OrdinaryObservation::Workspace(observation))
                    },
                }
            },
            OrdinarySource::Skill => {
                let Some(connection) = skill_references.as_mut() else {
                    continue;
                };
                match poll_reference_source(
                    events,
                    connection,
                    context,
                    |connection, context| connection.poll_ready(context),
                    |connection| connection.poll(),
                    |observation| matches!(observation, SkillReferencePoll::Pending),
                ) {
                    ReferenceSourcePoll::Pending => None,
                    ReferenceSourcePoll::Reselect => {
                        must_reselect = true;
                        None
                    },
                    ReferenceSourcePoll::Termination => return Ok(OrdinaryPoll::Termination),
                    ReferenceSourcePoll::Ready(observation) => {
                        Some(OrdinaryObservation::Skill(observation))
                    },
                }
            },
        };
        if let Some(observation) = observation {
            return Ok(OrdinaryPoll::Ready {
                source,
                observation,
            });
        }
    }

    Ok(if must_reselect {
        OrdinaryPoll::Reselect
    } else {
        OrdinaryPoll::Pending
    })
}

fn poll_reference_source<E, T, C, P, FReady, FPoll, FPending>(
    events: &mut UnixEventReader<E, T>,
    connection: &mut C,
    context: &mut Context<'_>,
    mut poll_ready: FReady,
    poll: FPoll,
    is_pending: FPending,
) -> ReferenceSourcePoll<P>
where
    E: crate::terminal::backend::unix::EventSource,
    T: TerminationSource,
    C: ?Sized,
    FReady: FnMut(&mut C, &mut Context<'_>) -> Poll<()>,
    FPoll: FnOnce(&mut C) -> Result<P, String>,
    FPending: FnOnce(&P) -> bool,
{
    let ready = poll_ready(connection, context);
    if events.poll_termination(context).is_ready() {
        return ReferenceSourcePoll::Termination;
    }
    if ready.is_pending() {
        return ReferenceSourcePoll::Pending;
    }

    let observation = poll(connection);
    if events.poll_termination(context).is_ready() {
        return ReferenceSourcePoll::Termination;
    }
    if let Ok(observation) = &observation
        && is_pending(observation)
    {
        let ready_again = poll_ready(connection, context).is_ready();
        if events.poll_termination(context).is_ready() {
            return ReferenceSourcePoll::Termination;
        }
        return if ready_again {
            ReferenceSourcePoll::Reselect
        } else {
            ReferenceSourcePoll::Pending
        };
    }

    ReferenceSourcePoll::Ready(observation)
}

struct OwnerThreadWake(Thread);

impl Wake for OwnerThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
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

pub(super) fn apply_agent_poll(
    state: &mut TuiState,
    observation: AgentPoll,
) -> Result<bool, LoopError> {
    match observation {
        AgentPoll::Pending => return Ok(false),
        AgentPoll::Record(record) => {
            state.observe_record(record).map_err(LoopError::State)?;
        },
        AgentPoll::RequestTrace(entry) => {
            state.observe_request_trace(entry);
        },
        AgentPoll::Durability(durability) => {
            state
                .observe_durability(durability)
                .map_err(LoopError::State)?;
        },
        AgentPoll::Submission(outcome) => {
            state
                .observe_submission_outcome(outcome)
                .map_err(LoopError::State)?;
        },
        AgentPoll::Closed => {
            return Err(LoopError::Agent(
                "the agent connection closed unexpectedly".to_owned(),
            ));
        },
    }
    Ok(true)
}

pub(super) fn apply_workspace_poll(
    state: &mut TuiState,
    connection: &mut Option<Box<dyn WorkspaceReferenceConnection>>,
    observation: Result<WorkspaceReferencePoll, String>,
) -> bool {
    match observation {
        Ok(WorkspaceReferencePoll::Pending) => false,
        Ok(WorkspaceReferencePoll::Update(update)) => matches!(
            state.observe_workspace_reference_update(update),
            StateEffect::Redraw
        ),
        Err(error) => {
            let changed = matches!(
                state.observe_workspace_reference_failure(error),
                StateEffect::Redraw
            );
            *connection = None;
            changed
        },
    }
}

pub(super) fn apply_skill_poll(
    state: &mut TuiState,
    connection: &mut Option<Box<dyn SkillReferenceConnection>>,
    observation: Result<SkillReferencePoll, String>,
) -> bool {
    match observation {
        Ok(SkillReferencePoll::Pending) => false,
        Ok(SkillReferencePoll::Update(update)) => matches!(
            state.observe_skill_reference_update(update),
            StateEffect::Redraw
        ),
        Err(error) => {
            let changed = matches!(
                state.observe_skill_reference_failure(error),
                StateEffect::Redraw
            );
            *connection = None;
            changed
        },
    }
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

#[allow(clippy::too_many_arguments)]
fn render_requested_frame<B, P>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    state: &mut TuiState,
    appearance: &crate::appearance::AppearanceState,
    size: Size,
    previous: &mut Option<Surface>,
    epoch: Instant,
    frames: &mut FrameScheduler,
    frame_visible: &mut bool,
    motion_deadline: &mut Option<Instant>,
) -> Result<(), LoopError>
where
    B: crate::terminal::backend::ScreenModeBackend
        + crate::terminal::backend::TerminalOutputBackend,
    B::Mode: PartialEq,
    P: LivePresenter<B>,
{
    let now = Instant::now();
    if size.width == 0 || size.height == 0 || !frames.is_due(now) {
        return Ok(());
    }
    *motion_deadline = redraw(session, viewport, state, appearance, size, previous, epoch)?;
    frames.rendered(Instant::now());
    *frame_visible = true;
    Ok(())
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

fn wait_timeout(
    base: Option<Duration>,
    motion_deadline: Option<Instant>,
    frame_deadline: Option<Instant>,
) -> Option<Duration> {
    wait_timeout_at(base, motion_deadline, frame_deadline, Instant::now())
}

fn wait_timeout_at(
    base: Option<Duration>,
    motion_deadline: Option<Instant>,
    frame_deadline: Option<Instant>,
    now: Instant,
) -> Option<Duration> {
    [motion_deadline, frame_deadline]
        .into_iter()
        .flatten()
        .map(|deadline| deadline.saturating_duration_since(now))
        .fold(base, |timeout, deadline| {
            Some(timeout.map_or(deadline, |current| current.min(deadline)))
        })
}

fn request_due_motion(
    frames: &mut FrameScheduler,
    frame_visible: bool,
    motion_deadline: &mut Option<Instant>,
    now: Instant,
) {
    if frame_visible && motion_deadline.is_some_and(|deadline| now >= deadline) {
        frames.request(FrameRequest::Coalesced);
        *motion_deadline = None;
    }
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

    use super::{next_motion_deadline, request_due_motion, wait_timeout_at};
    use crate::runner::frame::{FrameRateLimit, FrameRequest, FrameScheduler};

    // backpressure와 frame·motion 마감이 모두 없으면 주기적 poll 없이 무기한 대기합니다.
    #[test]
    fn idle_without_deadlines_has_no_timeout() {
        assert_eq!(wait_timeout_at(None, None, None, Instant::now()), None);
    }

    // 10ms worker 재시도보다 4ms 뒤 motion 마감이 더 가까우면 실제 sleep 없이도
    // scheduler가 정확히 4ms를 선택함을 결정적으로 확인한다.
    #[test]
    fn nearer_motion_deadline_shortens_the_base_wait() {
        let now = Instant::now();

        assert_eq!(
            wait_timeout_at(
                Some(Duration::from_millis(10)),
                now.checked_add(Duration::from_millis(4)),
                None,
                now,
            ),
            Some(Duration::from_millis(4))
        );
    }

    // frame 마감은 backpressure 재시도와 motion 마감보다 가까운 경우 owner wait의
    // 최솟값으로 선택되어 ordinary coalescing 경계를 넘지 않습니다.
    #[test]
    fn nearer_frame_deadline_shortens_the_combined_wait() {
        let now = Instant::now();

        assert_eq!(
            wait_timeout_at(
                Some(Duration::from_millis(10)),
                now.checked_add(Duration::from_millis(7)),
                now.checked_add(Duration::from_millis(3)),
                now,
            ),
            Some(Duration::from_millis(3))
        );
    }

    // 이미 지난 deadline은 음수 duration으로 변환되지 않고 zero로 포화되어 즉시
    // 재선택하게 하며, 현재 시각 이후의 다른 마감값을 잘못 기다리지 않습니다.
    #[test]
    fn overdue_deadlines_saturate_to_zero_wait() {
        let now = Instant::now();
        let past = now.checked_sub(Duration::from_millis(1)).unwrap();

        assert_eq!(
            wait_timeout_at(Some(Duration::from_millis(10)), Some(past), None, now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            wait_timeout_at(Some(Duration::from_millis(10)), None, Some(past), now),
            Some(Duration::ZERO)
        );
    }

    // 16ms motion tick을 60fps frame 요청으로 승격하면 지난 motion deadline은 소비되어
    // 남은 frame limiter 간격 동안 zero-timeout busy loop를 만들지 않습니다.
    #[test]
    fn due_motion_waits_for_the_remaining_60_fps_frame_interval() {
        let started = Instant::now();
        let motion_tick = started + Duration::from_millis(16);
        let mut frames = FrameScheduler::new(FrameRateLimit::Fps60);
        frames.request(FrameRequest::Immediate);
        frames.rendered(started);
        let mut motion_deadline = Some(motion_tick);

        request_due_motion(&mut frames, true, &mut motion_deadline, motion_tick);
        let timeout = wait_timeout_at(
            None,
            motion_deadline,
            frames.deadline(motion_tick),
            motion_tick,
        );

        assert!(timeout.is_some_and(|timeout| timeout > Duration::ZERO));
        assert!(timeout.is_some_and(|timeout| timeout < Duration::from_millis(1)));
    }

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
