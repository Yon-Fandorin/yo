use std::{
    error::Error,
    panic::AssertUnwindSafe,
    sync::Arc,
    task::{Context, Wake, Waker},
    thread::{self, Thread},
    time::Instant,
};

use yo_core::SubmissionOutcome;

use self::finalize::{LiveCleanup, LiveRunReport, finish};
use crate::{
    appearance::{ColorCapability, MotionPreference},
    runner::{
        AgentConnection, DispatchOutcome, PresentationMode, RunError, TerminalOutcome,
        TerminationSource, TuiSession,
        frame::{FrameRequest, FrameScheduler},
        session::SessionParts,
        source_schedule::SourceSchedule,
        state::{FrameError, StateEffect, StateError},
    },
    surface::Size,
    terminal::{
        backend::unix::{
            CrosstermEventSource, DirectTerminalWriter, RustixTermiosDriver, TtyStateAdapter,
            UnixBackend, UnixBackendError, UnixEventReader, terminal_size,
        },
        mode::{
            TerminalSession,
            fullscreen::{FullscreenRenderError, FullscreenViewport},
            inline::{InlineRenderError, InlineViewport},
            panic_route::catch_owner_panic,
            screen::{ScreenMode, enter_screen, run_fullscreen_guarded, run_inline_guarded},
        },
    },
};

mod finalize;
mod presenter;
mod sources;
mod timing;

pub(super) trait FrameViewport {
    fn invalidate_frame(&mut self);

    fn abandon_frame(&mut self) {
        self.invalidate_frame();
    }
}

#[cfg(test)]
pub(super) use presenter::RenderReceipt;
pub(super) use presenter::{LivePresenter, prepare_resize};
pub(super) use sources::{
    OrdinaryObservation, OrdinaryPoll, apply_agent_poll, apply_skill_poll, apply_workspace_poll,
    handle_backpressured_input, poll_ordinary,
};

use self::{
    presenter::{PresentationState, render_requested_frame},
    sources::{dispatch_skill_search, dispatch_workspace_search},
    timing::{WORKER_RETRY_INTERVAL, request_due_motion, wait_timeout},
};

pub(super) type LiveBackendError = UnixBackendError<rustix::io::Errno>;

#[derive(Clone, Copy)]
pub(super) struct GenerationStart {
    size: Size,
    started: Instant,
}

impl GenerationStart {
    pub(super) const fn new(size: Size, started: Instant) -> Self {
        Self { size, started }
    }
}

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
    GeometryEpochOverflow,
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
            Self::State(StateError::StalePublication) => {
                "acknowledging a stale transcript publication failed".to_owned()
            },
            Self::Frame(error) => error.detail(),
            Self::InlineRender(error) => format!("rendering the inline frame failed: {error}"),
            Self::FullscreenRender(error) => {
                format!("rendering the fullscreen frame failed: {error}")
            },
            Self::GeometryEpochOverflow => {
                "tracking terminal geometry changes overflowed".to_owned()
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

    let output = DirectTerminalWriter::stdout();
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
                    GenerationStart::new(size, started),
                    &mut terminal_size,
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
                    GenerationStart::new(size, started),
                    &mut terminal_size,
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

pub(super) fn drive<B, E, T, A, P, G, GE>(
    session: &mut TerminalSession<'_, B>,
    viewport: &mut P,
    events: &mut UnixEventReader<E, T>,
    retained: &mut TuiSession,
    agent: &mut A,
    start: GenerationStart,
    sample_geometry: &mut G,
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
    G: FnMut() -> Result<Size, GE>,
    GE: std::fmt::Display,
{
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
        publication_recovery_evidence,
    } = retained.parts_mut();
    let mut presentation =
        PresentationState::new(start.size, start.started, publication_recovery_evidence);
    let mut frames = FrameScheduler::new(frame_rate_limit);
    let mut source_schedule = SourceSchedule::default();
    frames.request(FrameRequest::Immediate);

    loop {
        request_due_motion(
            &mut frames,
            presentation.frame_visible,
            &mut presentation.motion_deadline,
            Instant::now(),
        );
        let mut observe_geometry = || {
            let resizes = events
                .observe_post_flush_resizes(&mut context)
                .map_err(|error| LoopError::Input(format!("{error:?}")))?;
            let sampled_size = sample_geometry().map_err(|error| {
                LoopError::Input(format!("reading terminal size failed: {error}"))
            })?;
            Ok(presenter::GeometryObservation {
                resize_count: resizes.count,
                sampled_size,
            })
        };
        render_requested_frame(
            session,
            viewport,
            state,
            appearance,
            &mut presentation,
            &mut frames,
            &mut observe_geometry,
        )?;

        if let Some(action) = pending_control.take() {
            let admission = agent
                .retry(action)
                .map_err(|error| LoopError::Agent(error.to_string()))?;
            let effect = apply_admission(state, pending_control, admission)?;
            if finish_admission_effect(effect, &mut frames) {
                return Ok(LoopExit::User);
            }
        }
        if pending_control.is_none()
            && let Some(action) = pending_dispatch.take()
        {
            let admission = agent
                .retry(action)
                .map_err(|error| LoopError::Agent(error.to_string()))?;
            let effect = apply_admission(state, pending_dispatch, admission)?;
            if finish_admission_effect(effect, &mut frames) {
                return Ok(LoopExit::User);
            }
        }
        let backpressured = pending_control.is_some() || pending_dispatch.is_some();
        let base = backpressured.then_some(WORKER_RETRY_INTERVAL);
        let timeout = wait_timeout(
            base,
            presentation.motion_deadline,
            frames.deadline(Instant::now()),
        );
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
                    if state.model_switch_ready() {
                        return Ok(LoopExit::User);
                    }
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
                        presentation.started.elapsed(),
                        pending_control.is_none(),
                    )
                } else {
                    state.handle(input, presentation.started.elapsed())
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
                        let admission = agent
                            .dispatch(action)
                            .map_err(|error| LoopError::Agent(error.to_string()))?;
                        let retained = if backpressured || is_interrupt {
                            &mut *pending_control
                        } else {
                            &mut *pending_dispatch
                        };
                        let effect = apply_admission(state, retained, admission)?;
                        if finish_admission_effect(effect, &mut frames) {
                            return Ok(LoopExit::User);
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
                        presentation.geometry_epoch = presentation
                            .geometry_epoch
                            .checked_add(1)
                            .ok_or(LoopError::GeometryEpochOverflow)?;
                        prepare_resize(viewport, &mut presentation.size, next);
                        presentation.frame_visible = false;
                        presentation.motion_deadline = None;
                        frames.request(FrameRequest::Immediate);
                    },
                }
            },
        }
    }
}

fn apply_admission(
    state: &mut crate::runner::state::TuiState,
    retained: &mut Option<crate::runner::PendingDispatch>,
    admission: DispatchOutcome,
) -> Result<StateEffect, LoopError> {
    match admission {
        DispatchOutcome::Queued => {
            *retained = None;
            Ok(StateEffect::Unchanged)
        },
        DispatchOutcome::Backpressured(pending) => {
            *retained = Some(pending);
            Ok(StateEffect::Unchanged)
        },
        DispatchOutcome::Rejected { id, rejection } => {
            *retained = None;
            state
                .observe_submission_outcome(SubmissionOutcome::Rejected { id, rejection })
                .map_err(LoopError::State)
        },
    }
}

fn finish_admission_effect(effect: StateEffect, frames: &mut FrameScheduler) -> bool {
    match effect {
        StateEffect::Unchanged => false,
        StateEffect::Redraw => {
            frames.request(FrameRequest::Coalesced);
            false
        },
        StateEffect::Exit => true,
        StateEffect::Dispatch(_)
        | StateEffect::WorkspaceSearch(_)
        | StateEffect::SkillSearch(_)
        | StateEffect::Resize(_)
        | StateEffect::Suspend => {
            unreachable!("submission admission cannot produce an unrelated state effect")
        },
    }
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
