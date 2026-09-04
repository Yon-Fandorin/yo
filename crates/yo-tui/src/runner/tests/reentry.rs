use std::{
    cell::Cell,
    collections::VecDeque,
    convert::Infallible,
    io,
    rc::Rc,
    sync::mpsc,
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{
    Event, KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent, KeyModifiers,
};
use yo_core::{
    AgentCommand, AgentEvent, AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll,
    BackendAdapter, BackendCapabilities, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendPoll, BackendResumeTarget, BackendStopHandle, CommandAdmission, TranscriptRecord,
    TurnOutcome, TurnRef, UserInput,
};

use crate::{
    appearance::{AppearanceCandidate, ColorCapability, GlyphProfile, MotionPreference},
    input::event::{
        InputEvent, KeyAction, KeyCode as YoKeyCode, KeyEvent as YoKeyEvent,
        KeyModifiers as YoKeyModifiers, KeyState,
    },
    runner::{
        AgentAction, AgentConnection, AgentPoll, DispatchOutcome, FrameRateLimit, PendingDispatch,
        TerminationEvent, TerminationSource, TuiSession,
        state::StateEffect,
        unix::{FrameViewport, GenerationStart, LivePresenter, LoopError, LoopExit, drive},
    },
    surface::{CellContent, Point, Size, Surface},
    terminal::{
        backend::{
            ScreenModeBackend, TerminalBackend, TerminalOutputBackend,
            unix::{EventSource, UnixEventReader},
        },
        mode::{
            TerminalSession,
            inline::InlineRecovery,
            screen::{ScreenMode, enter_screen},
        },
    },
};

mod publication;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    BracketedPaste,
    AlternateScreen,
    CursorVisibility,
}

#[derive(Default)]
struct Backend {
    output: Vec<u8>,
}

impl TerminalBackend for Backend {
    type TtyState = ();
    type Mode = Mode;
    type Error = Infallible;

    fn capture_tty_state(&mut self) -> Result<Self::TtyState, Self::Error> {
        Ok(())
    }

    fn enable_raw_input(&mut self, _original: &Self::TtyState) -> Result<(), Self::Error> {
        Ok(())
    }

    fn acquire_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
        Ok(())
    }

    fn release_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_tty_state(&mut self, _state: &Self::TtyState) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ScreenModeBackend for Backend {
    fn bracketed_paste_mode() -> Self::Mode {
        Mode::BracketedPaste
    }

    fn alternate_screen_mode() -> Self::Mode {
        Mode::AlternateScreen
    }

    fn cursor_visibility_mode() -> Self::Mode {
        Mode::CursorVisibility
    }
}

impl TerminalOutputBackend for Backend {
    type Output = Vec<u8>;

    fn output(&mut self) -> &mut Self::Output {
        &mut self.output
    }
}

struct Events {
    events: VecDeque<Event>,
    polls: Rc<Cell<usize>>,
    reads: Rc<Cell<usize>>,
}

impl Events {
    fn new(
        events: impl IntoIterator<Item = Event>,
        polls: Rc<Cell<usize>>,
        reads: Rc<Cell<usize>>,
    ) -> Self {
        Self {
            events: events.into_iter().collect(),
            polls,
            reads,
        }
    }
}

impl EventSource for Events {
    type Error = io::Error;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        self.polls.set(self.polls.get() + 1);
        self.events.pop_front().map_or(Poll::Pending, |event| {
            self.reads.set(self.reads.get() + 1);
            Poll::Ready(Ok(event))
        })
    }
}

struct EventAt {
    event: Option<Event>,
    ready_at: Instant,
    polls: Rc<Cell<usize>>,
    reads: Rc<Cell<usize>>,
}

impl EventAt {
    fn new(
        event: Event,
        ready_at: Instant,
        polls: Rc<Cell<usize>>,
        reads: Rc<Cell<usize>>,
    ) -> Self {
        Self {
            event: Some(event),
            ready_at,
            polls,
            reads,
        }
    }
}

impl EventSource for EventAt {
    type Error = io::Error;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        self.polls.set(self.polls.get() + 1);
        let Some(event) = self.event.take() else {
            return Poll::Pending;
        };
        thread::sleep(self.ready_at.saturating_duration_since(Instant::now()));
        self.reads.set(self.reads.get() + 1);
        Poll::Ready(Ok(event))
    }
}

struct WakingEventAt {
    event: Option<Event>,
    ready_at: Instant,
    armed: bool,
}

struct DelayedEventSequence {
    immediate: VecDeque<Event>,
    delayed: Option<(Instant, Event)>,
    armed: bool,
    polls: Rc<Cell<usize>>,
    reads: Rc<Cell<usize>>,
}

impl DelayedEventSequence {
    fn new(
        immediate: impl IntoIterator<Item = Event>,
        delayed: (Instant, Event),
        polls: Rc<Cell<usize>>,
        reads: Rc<Cell<usize>>,
    ) -> Self {
        Self {
            immediate: immediate.into_iter().collect(),
            delayed: Some(delayed),
            armed: false,
            polls,
            reads,
        }
    }
}

impl EventSource for DelayedEventSequence {
    type Error = io::Error;

    fn poll_event(&mut self, context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        self.polls.set(self.polls.get() + 1);
        if let Some(event) = self.immediate.pop_front() {
            self.reads.set(self.reads.get() + 1);
            return Poll::Ready(Ok(event));
        }
        let Some((ready_at, _)) = self.delayed.as_ref() else {
            return Poll::Pending;
        };
        let now = Instant::now();
        if now >= *ready_at {
            let (_, event) = self.delayed.take().unwrap();
            self.reads.set(self.reads.get() + 1);
            return Poll::Ready(Ok(event));
        }
        if !self.armed {
            self.armed = true;
            let delay = ready_at.saturating_duration_since(now);
            let wake = context.waker().clone();
            thread::spawn(move || {
                thread::sleep(delay);
                wake.wake();
            });
        }
        Poll::Pending
    }
}

impl WakingEventAt {
    fn new(event: Event, ready_at: Instant) -> Self {
        Self {
            event: Some(event),
            ready_at,
            armed: false,
        }
    }
}

impl EventSource for WakingEventAt {
    type Error = io::Error;

    fn poll_event(&mut self, context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        let Some(_) = self.event.as_ref() else {
            return Poll::Pending;
        };
        let now = Instant::now();
        if now >= self.ready_at {
            return Poll::Ready(Ok(self.event.take().unwrap()));
        }
        if !self.armed {
            self.armed = true;
            let delay = self.ready_at.saturating_duration_since(now);
            let wake = context.waker().clone();
            thread::spawn(move || {
                thread::sleep(delay);
                wake.wake();
            });
        }
        Poll::Pending
    }
}

struct ResizeOnce {
    resized: Rc<Cell<bool>>,
    emitted: bool,
}

impl EventSource for ResizeOnce {
    type Error = io::Error;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        if self.emitted {
            Poll::Pending
        } else {
            self.emitted = true;
            self.resized.set(true);
            Poll::Ready(Ok(Event::Resize(32, 8)))
        }
    }
}

struct TerminateAfterResize {
    resized: Rc<Cell<bool>>,
    observed_after_resize: bool,
}

impl TerminationSource for TerminateAfterResize {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        if !self.resized.get() {
            return Poll::Pending;
        }
        if !self.observed_after_resize {
            self.observed_after_resize = true;
            Poll::Pending
        } else {
            Poll::Ready(TerminationEvent::Requested)
        }
    }
}

struct StopAfter {
    counter: Rc<Cell<usize>>,
    threshold: usize,
}

impl TerminationSource for StopAfter {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        if self.counter.get() >= self.threshold {
            Poll::Ready(TerminationEvent::Requested)
        } else {
            Poll::Pending
        }
    }
}

struct StopAfterOrWatchdog {
    counter: Rc<Cell<usize>>,
    threshold: usize,
    deadline: Instant,
    armed: bool,
    watchdog_fired: Rc<Cell<bool>>,
}

impl TerminationSource for StopAfterOrWatchdog {
    fn poll_termination(&mut self, context: &mut Context<'_>) -> Poll<TerminationEvent> {
        if self.counter.get() >= self.threshold {
            return Poll::Ready(TerminationEvent::Requested);
        }
        let now = Instant::now();
        if now >= self.deadline {
            self.watchdog_fired.set(true);
            return Poll::Ready(TerminationEvent::Requested);
        }
        if !self.armed {
            self.armed = true;
            let delay = self.deadline.saturating_duration_since(now);
            let wake = context.waker().clone();
            thread::spawn(move || {
                thread::sleep(delay);
                wake.wake();
            });
        }
        Poll::Pending
    }
}

#[derive(Default)]
struct SimpleAgent {
    records: VecDeque<TranscriptRecord>,
}

#[derive(Default)]
struct RetainingAgent {
    retries: usize,
}

impl AgentConnection for RetainingAgent {
    type Error = Infallible;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        self.retries += 1;
        Ok(DispatchOutcome::Backpressured(pending))
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

impl AgentConnection for SimpleAgent {
    type Error = Infallible;

    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        if let AgentIntent::Submit(input) = action {
            self.records.push_back(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: TurnRef::new(
                        "01890f00-0000-7000-8000-000000000001"
                            .parse()
                            .expect("the fixture is a UUIDv7"),
                        yo_core::TurnId::new(std::num::NonZeroU64::MIN),
                    ),
                    input: input.into_input(),
                },
            ));
        }
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(self
            .records
            .pop_front()
            .map_or(AgentPoll::Pending, AgentPoll::Record))
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

#[derive(Default)]
struct Presenter {
    invalidations: usize,
    previous_on_render: Vec<bool>,
    frames: Vec<Surface>,
    publications: Vec<Surface>,
    render_count: Rc<Cell<usize>>,
    recovery: Option<InlineRecovery>,
}

impl FrameViewport for Presenter {
    fn invalidate_frame(&mut self) {
        self.invalidations += 1;
    }
}

impl LivePresenter<Backend> for Presenter {
    fn render(
        &mut self,
        _session: &mut TerminalSession<'_, Backend>,
        previous: Option<&Surface>,
        current: &Surface,
        _cursor: Point,
        publication: Option<&Surface>,
        _terminal_size: Size,
    ) -> Result<super::super::unix::RenderReceipt, LoopError> {
        self.previous_on_render.push(previous.is_some());
        self.frames.push(current.clone());
        if let Some(publication) = publication {
            self.publications.push(publication.clone());
        }
        self.render_count.set(self.render_count.get() + 1);
        Ok(super::super::unix::RenderReceipt {
            publication_complete: publication.is_some(),
            publication_recovery: publication.and(self.recovery),
        })
    }
}

fn surface_text(surface: &Surface) -> String {
    let size = surface.size();
    (0..size.height)
        .map(|y| {
            (0..size.width)
                .filter_map(
                    |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank | CellContent::Continuation { .. } => Some(' '),
                        CellContent::Grapheme { text, .. } => text.chars().next(),
                    },
                )
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn styles_for_ascii_text(surface: &Surface, needle: &str) -> Vec<crate::surface::Style> {
    let expected = needle.chars().collect::<Vec<_>>();
    let size = surface.size();
    for y in 0..size.height {
        for x in 0..size.width {
            let fits = expected.iter().enumerate().all(|(offset, expected)| {
                let Some(point_x) = x.checked_add(u16::try_from(offset).unwrap()) else {
                    return false;
                };
                matches!(
                    surface.cell(Point::new(point_x, y)).map(|cell| cell.content()),
                    Some(CellContent::Grapheme { text, .. }) if text.starts_with(*expected)
                )
            });
            if fits {
                return expected
                    .iter()
                    .enumerate()
                    .map(|(offset, _)| {
                        surface
                            .cell(Point::new(x + u16::try_from(offset).unwrap(), y))
                            .unwrap()
                            .style()
                    })
                    .collect();
            }
        }
    }
    panic!("expected ASCII text was not rendered: {needle}");
}

fn key(code: CrosstermKeyCode) -> Event {
    Event::Key(CrosstermKeyEvent::new(code, KeyModifiers::NONE))
}

fn wait_for_session_change(session: &mut AgentSession) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match session.poll().unwrap() {
            AgentSessionPoll::Changed => return,
            AgentSessionPoll::Pending if Instant::now() < deadline => {
                thread::yield_now();
            },
            other => panic!("worker did not publish the expected change: {other:?}"),
        }
    }
}

fn dispatch_until_queued(session: &mut AgentSession, intent: AgentIntent) {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut admission = session.dispatch(intent).unwrap();
    loop {
        match admission {
            CommandAdmission::Queued => return,
            CommandAdmission::Backpressured(pending) if Instant::now() < deadline => {
                thread::yield_now();
                admission = session.retry(pending).unwrap();
            },
            other => panic!("worker did not admit the command before the deadline: {other:?}"),
        }
    }
}

fn run_generation(
    retained: &mut TuiSession,
    agent: &mut impl AgentConnection,
    events: Events,
    stop_after_frames: usize,
) -> Presenter {
    run_generation_at(retained, agent, events, stop_after_frames, Instant::now())
}

fn run_generation_at(
    retained: &mut TuiSession,
    agent: &mut impl AgentConnection,
    events: Events,
    stop_after_frames: usize,
    started: Instant,
) -> Presenter {
    let render_count = Rc::new(Cell::new(0));
    let termination = StopAfter {
        counter: Rc::clone(&render_count),
        threshold: stop_after_frames,
    };
    run_generation_with_termination_at(retained, agent, events, termination, render_count, started)
}

fn run_generation_with_termination_at<E, T>(
    retained: &mut TuiSession,
    agent: &mut impl AgentConnection,
    events: E,
    termination: T,
    render_count: Rc<Cell<usize>>,
    started: Instant,
) -> Presenter
where
    E: EventSource<Error = io::Error>,
    T: TerminationSource,
{
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let mut presenter = Presenter {
        render_count,
        ..Presenter::default()
    };
    let mut reader = UnixEventReader::new(events, termination);

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            retained,
            agent,
            GenerationStart::new(Size::new(16, 6), started),
            &mut || Ok::<Size, Infallible>(Size::new(16, 6)),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();
    presenter
}

fn retry_representable_past<T>(
    timeout: Duration,
    mut sample: impl FnMut() -> Option<T>,
) -> Result<T, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = sample() {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the monotonic clock cannot represent the requested test history within {timeout:?}"
            ));
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn representable_past(offset: Duration) -> Instant {
    retry_representable_past(offset + Duration::from_millis(100), || {
        Instant::now().checked_sub(offset)
    })
    .unwrap_or_else(|error| panic!("motion test clock prerequisite failed: {error}"))
}

fn turn() -> TurnRef {
    TurnRef::new(
        "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("the fixture is a UUIDv7"),
        yo_core::TurnId::new(std::num::NonZeroU64::MIN),
    )
}

fn active_motion_session(period: Duration) -> TuiSession {
    let mut retained = TuiSession::with_glyph_profile(
        GlyphProfile::Ascii,
        ColorCapability::Unknown,
        MotionPreference::Standard,
    );
    let candidate = AppearanceCandidate::for_profile(GlyphProfile::Ascii)
        .with_activity_motion_for_test(period, period, &["*"])
        .unwrap()
        .with_activity_sweep_period_for_test(period.saturating_mul(2))
        .unwrap();
    retained.commit_appearance(candidate).unwrap();
    retained
        .parts_mut()
        .state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    retained
}

struct BlockingAgentBackend {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    processed: mpsc::Sender<()>,
    blocked: bool,
}

struct FinishingBlockingAgentBackend {
    entered: mpsc::Sender<TurnRef>,
    release: mpsc::Receiver<()>,
    pending_finish: Option<TurnRef>,
}

impl BackendAdapter for FinishingBlockingAgentBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        BackendStopHandle::no_op()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none().with_steer()
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if let AgentCommand::StartTurn { turn, .. } = command {
            self.pending_finish = Some(turn);
            self.entered.send(turn).unwrap();
            self.release.recv().unwrap();
        }
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        Ok(self
            .pending_finish
            .take()
            .map_or(BackendPoll::Pending, |turn| {
                BackendPoll::Event(BackendEvent::TurnFinished {
                    turn,
                    outcome: TurnOutcome::Completed,
                })
            }))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

impl BackendAdapter for BlockingAgentBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        BackendStopHandle::no_op()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none().with_steer()
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.processed.send(()).unwrap();
        if matches!(command, AgentCommand::StartTurn { .. }) && !self.blocked {
            self.blocked = true;
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        Ok(BackendPoll::Pending)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

struct CoreAgent {
    session: AgentSession,
    retries: usize,
}

impl AgentConnection for CoreAgent {
    type Error = AgentSessionError;

    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        self.session.dispatch(action)
    }

    fn retry(&mut self, pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        self.retries += 1;
        self.session.retry(pending)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.session.poll_ready(context)
    }
}

// 첫 generation의 실제 input·render loop가 만든 대화와 작성 중인 prompt를 같은
// TuiSession의 두 번째 generation이 다시 그리며, 이전 frame은 재사용하지 않는다.
#[test]
fn second_terminal_generation_renders_retained_state_from_a_fresh_frame() {
    let mut retained = TuiSession::with_glyph_profile(
        GlyphProfile::Ascii,
        ColorCapability::Unknown,
        MotionPreference::Standard,
    );
    let appearance_revision = retained.appearance_pin().revision().get();
    let mut agent = SimpleAgent::default();
    let first_polls = Rc::new(Cell::new(0));
    let first = run_generation(
        &mut retained,
        &mut agent,
        Events::new(
            [
                Event::Paste("question".to_owned()),
                key(CrosstermKeyCode::Enter),
                Event::Paste("draft".to_owned()),
            ],
            Rc::clone(&first_polls),
            Rc::new(Cell::new(0)),
        ),
        2,
    );

    let second_polls = Rc::new(Cell::new(0));
    let second = run_generation(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&second_polls), Rc::new(Cell::new(0))),
        1,
    );

    assert!(!first.previous_on_render[0]);
    assert_eq!(second.previous_on_render, [false]);
    assert!(
        first
            .frames
            .iter()
            .any(|frame| surface_text(frame).contains("> question"))
    );
    assert!(surface_text(&second.frames[0]).contains("> question"));
    assert!(surface_text(&second.frames[0]).contains("draft"));
    assert_eq!(
        retained.appearance_pin().revision().get(),
        appearance_revision
    );
}

// 실제 yo-core backpressure에서 얻은 두 작업을 TuiSession의 두 slot에 보관하면
// 다음 generation이 둘을 재시도하고 성공 뒤 두 slot을 모두 비운다.
#[test]
fn next_terminal_generation_retries_both_retained_backpressure_slots() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (processed_tx, processed_rx) = mpsc::channel();
    let backend = BlockingAgentBackend {
        entered: entered_tx,
        release: release_rx,
        processed: processed_tx,
        blocked: false,
    };
    let session = AgentSession::start(backend).unwrap();
    let transcript = session.transcript_reader();
    let mut agent = CoreAgent {
        session,
        retries: 0,
    };
    processed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    wait_for_session_change(&mut agent.session);

    dispatch_until_queued(
        &mut agent.session,
        AgentIntent::submit("block".to_owned()).unwrap(),
    );
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    processed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let CommandAdmission::Backpressured(normal) = agent
        .dispatch(AgentIntent::submit("normal-pending".to_owned()).unwrap())
        .unwrap()
    else {
        panic!("the full normal lane must retain the next command");
    };
    let CommandAdmission::Backpressured(control) = agent
        .dispatch(AgentIntent::submit("control-slot-pending".to_owned()).unwrap())
        .unwrap()
    else {
        panic!("the full normal lane must retain another command");
    };

    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    {
        let parts = retained.parts_mut();
        *parts.pending_dispatch = Some(normal);
        *parts.pending_control = Some(control);
    }

    release_tx.send(()).unwrap();
    wait_for_session_change(&mut agent.session);
    assert!(transcript.read_after(None).entries().iter().any(|entry| {
        matches!(
            entry.record(),
            TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { .. })
        )
    }));
    let polls = Rc::new(Cell::new(0));
    run_generation(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&polls), Rc::new(Cell::new(0))),
        1,
    );

    assert_eq!(agent.retries, 2);
    let parts = retained.parts_mut();
    assert!(parts.pending_control.is_none());
    assert!(parts.pending_dispatch.is_none());
    agent.session.shutdown().unwrap();
}

// 이전 generation에서 exact Turn steer가 backpressure로 남은 뒤 그 Turn이 끝나도, 다음
// generation의 재시도는 terminal을 죽이지 않고 같은 ID의 거절을 표시하며 draft를 보존한다.
#[test]
fn next_generation_recovers_a_stale_retained_steer_as_a_submission_rejection() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let backend = FinishingBlockingAgentBackend {
        entered: entered_tx,
        release: release_rx,
        pending_finish: None,
    };
    let session = AgentSession::start(backend).unwrap();
    let transcript = session.transcript_reader();
    let mut agent = CoreAgent {
        session,
        retries: 0,
    };
    wait_for_session_change(&mut agent.session);
    dispatch_until_queued(
        &mut agent.session,
        AgentIntent::submit("working".to_owned()).unwrap(),
    );
    let observed_turn = entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    let pending = {
        let state = retained.parts_mut().state;
        state
            .observe(AgentEvent::TurnStarted {
                turn: observed_turn,
            })
            .unwrap();
        state
            .handle(InputEvent::Paste("late steer".to_owned()), Duration::ZERO)
            .unwrap();
        let StateEffect::Dispatch(action) = state
            .handle(
                InputEvent::Key(YoKeyEvent {
                    code: YoKeyCode::Enter,
                    modifiers: YoKeyModifiers::NONE,
                    action: KeyAction::Press,
                    state: KeyState::NONE,
                }),
                Duration::ZERO,
            )
            .unwrap()
        else {
            panic!("active prompt input must create an exact-Turn steer");
        };
        assert!(matches!(
            &action,
            AgentAction::Steer { turn, submission }
                if *turn == observed_turn && submission.input().as_str() == "late steer"
        ));
        let CommandAdmission::Backpressured(pending) = agent.dispatch(action).unwrap() else {
            panic!("the provider-held runtime lock must retain the exact steer");
        };
        pending
    };
    *retained.parts_mut().pending_dispatch = Some(pending);

    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if transcript.read_after(None).entries().iter().any(|entry| {
            matches!(
                entry.record(),
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, .. })
                    if *turn == observed_turn
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the core Session did not finish the retained steer target"
        );
        thread::yield_now();
    }
    retained
        .parts_mut()
        .state
        .observe(AgentEvent::TurnFinished {
            turn: observed_turn,
            outcome: TurnOutcome::Completed,
        })
        .unwrap();

    let polls = Rc::new(Cell::new(0));
    run_generation(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&polls), Rc::new(Cell::new(0))),
        2,
    );

    assert_eq!(agent.retries, 1);
    let parts = retained.parts_mut();
    assert!(parts.pending_dispatch.is_none());
    assert_eq!(parts.state.editor().text(), "late steer");
    assert!(parts.state.transcript().items().iter().any(|item| {
        matches!(
            item.body(),
            crate::transcript::TranscriptBody::Message(message)
                if message.text().contains("Submission rejected")
        )
    }));
    agent.session.shutdown().unwrap();
}

// motion 마감과 paste 입력이 같은 wakeup에서 만났을 때 입력 결과만 한 번 그린다.
// 따라서 시간 tick용 중간 frame이 중복으로 끼어들지 않는다.
#[test]
fn normal_wait_coalesces_input_and_due_motion_into_one_redraw() {
    let period = Duration::from_millis(100);
    let mut retained = active_motion_session(period);
    let mut agent = SimpleAgent::default();
    let polls = Rc::new(Cell::new(0));
    let reads = Rc::new(Cell::new(0));
    let started = Instant::now();
    let render_count = Rc::new(Cell::new(0));
    let presenter = run_generation_with_termination_at(
        &mut retained,
        &mut agent,
        EventAt::new(
            Event::Paste("x".to_owned()),
            started + period,
            Rc::clone(&polls),
            Rc::clone(&reads),
        ),
        StopAfter {
            counter: Rc::clone(&render_count),
            threshold: 2,
        },
        render_count,
        started,
    );

    assert_eq!(presenter.frames.len(), 2);
    assert_eq!(reads.get(), 1);
    assert!(!surface_text(&presenter.frames[0]).contains('x'));
    assert!(surface_text(&presenter.frames[1]).contains('x'));
    assert_eq!(
        styles_for_ascii_text(&presenter.frames[0], "* Working")[0].attributes,
        crate::surface::Attributes::DIM
    );
    assert_eq!(
        styles_for_ascii_text(&presenter.frames[1], "* Working")[0].attributes,
        crate::surface::Attributes::BOLD
    );
}

fn run_scheduled_motion_input(
    period: Duration,
    input_after: Duration,
    stop_after_frames: usize,
) -> Presenter {
    let mut retained = active_motion_session(period);
    let mut agent = SimpleAgent::default();
    let started = Instant::now();
    let render_count = Rc::new(Cell::new(0));
    run_generation_with_termination_at(
        &mut retained,
        &mut agent,
        WakingEventAt::new(Event::Paste("x".to_owned()), started + input_after),
        StopAfter {
            counter: Rc::clone(&render_count),
            threshold: stop_after_frames,
        },
        render_count,
        started,
    )
}

// Motion deadline보다 충분히 먼저 준비된 input은 그 자체의 두 번째 frame을 만들고,
// 아직 due가 아닌 motion frame을 앞당기지 않습니다.
#[test]
fn input_before_motion_deadline_is_one_input_redraw() {
    let presenter =
        run_scheduled_motion_input(Duration::from_millis(200), Duration::from_millis(100), 2);

    assert_eq!(presenter.frames.len(), 2);
    assert!(!surface_text(&presenter.frames[0]).contains('x'));
    assert!(surface_text(&presenter.frames[1]).contains('x'));
}

// Motion deadline 뒤에 준비된 input은 먼저 due-motion frame을 관찰한 다음 별도의 input
// frame을 만들어 coincident case의 두-frame assertion과 구별됩니다.
#[test]
fn input_after_motion_deadline_follows_the_motion_redraw() {
    let presenter =
        run_scheduled_motion_input(Duration::from_millis(200), Duration::from_millis(300), 3);

    assert_eq!(presenter.frames.len(), 3);
    assert!(!surface_text(&presenter.frames[1]).contains('x'));
    assert!(surface_text(&presenter.frames[2]).contains('x'));
}

// 첫 frame 직후 ordinary coalescing 간격 안에 resize가 들어오면 viewport를 무효화하고
// resize frame을 즉시 그려, 기존 frame limiter가 correctness frame을 지연시키지 않는다.
#[test]
fn resize_frame_is_immediate_and_invalidates_the_previous_viewport() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard)
        .with_frame_rate_limit(FrameRateLimit::Fps60);
    let mut agent = SimpleAgent::default();
    let resized = Rc::new(Cell::new(false));
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let render_count = Rc::new(Cell::new(0));
    let mut presenter = Presenter {
        render_count,
        ..Presenter::default()
    };
    let mut reader = UnixEventReader::new(
        ResizeOnce {
            resized: Rc::clone(&resized),
            emitted: false,
        },
        TerminateAfterResize {
            resized,
            observed_after_resize: false,
        },
    );

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            &mut retained,
            &mut agent,
            GenerationStart::new(Size::new(16, 6), Instant::now()),
            &mut || Ok::<Size, Infallible>(Size::new(16, 6)),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();

    assert_eq!(presenter.frames.len(), 2);
    assert_eq!(presenter.invalidations, 1);
    assert_eq!(presenter.previous_on_render, [false, true]);
}

// 전송 재시도 중 10ms backpressure poll이 motion 마감을 지나더라도 이를 놓치지 않고
// 실제 marker frame을 다시 그린다. OS sleep 오차에 따른 더 짧은 timeout은 요구하지 않는다.
#[test]
fn backpressure_wait_keeps_visible_motion_deadline() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (processed_tx, processed_rx) = mpsc::channel();
    let backend = BlockingAgentBackend {
        entered: entered_tx,
        release: release_rx,
        processed: processed_tx,
        blocked: false,
    };
    let mut core = AgentSession::start(backend).unwrap();
    processed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    wait_for_session_change(&mut core);
    dispatch_until_queued(&mut core, AgentIntent::submit("block".to_owned()).unwrap());
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    processed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let CommandAdmission::Backpressured(pending) = core
        .dispatch(AgentIntent::submit("pending".to_owned()).unwrap())
        .unwrap()
    else {
        panic!("the full normal lane must return an opaque pending dispatch");
    };

    let period = Duration::from_millis(16);
    let mut retained = active_motion_session(period);
    *retained.parts_mut().pending_dispatch = Some(pending);
    let mut agent = RetainingAgent::default();
    let polls = Rc::new(Cell::new(0));
    let render_count = Rc::new(Cell::new(0));
    let watchdog_fired = Rc::new(Cell::new(false));
    let started = Instant::now();
    let presenter = run_generation_with_termination_at(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&polls), Rc::new(Cell::new(0))),
        StopAfterOrWatchdog {
            counter: Rc::clone(&render_count),
            threshold: 2,
            deadline: started + Duration::from_secs(1),
            armed: false,
            watchdog_fired: Rc::clone(&watchdog_fired),
        },
        render_count,
        started,
    );

    release_tx.send(()).unwrap();
    wait_for_session_change(&mut core);
    core.shutdown().unwrap();

    assert!(agent.retries >= 2);
    assert!(presenter.frames.len() >= 2);
    assert!(!watchdog_fired.get());
    assert!(surface_text(&presenter.frames[0]).contains("* Working"));
}

// terminal이 잠시 0x0이 되어 frame을 숨겨도 generation epoch는 유지한다.
// 다시 보이는 순간에는 resize 시점부터가 아니라 원래 경과 시간의 style phase를 고른다.
#[test]
fn zero_size_resize_preserves_the_generation_motion_epoch() {
    let period = Duration::from_secs(1);
    let mut retained = active_motion_session(period);
    let mut agent = SimpleAgent::default();
    let polls = Rc::new(Cell::new(0));
    let events = Events::new(
        [Event::Resize(16, 6)],
        Rc::clone(&polls),
        Rc::new(Cell::new(0)),
    );
    let render_count = Rc::new(Cell::new(0));
    let termination = StopAfter {
        counter: Rc::clone(&render_count),
        threshold: 1,
    };
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let mut presenter = Presenter {
        render_count,
        ..Presenter::default()
    };
    let mut reader = UnixEventReader::new(events, termination);
    let started = representable_past(period + Duration::from_millis(100));

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            &mut retained,
            &mut agent,
            GenerationStart::new(Size::new(0, 0), started),
            &mut || Ok::<Size, Infallible>(Size::new(16, 6)),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();

    assert_eq!(presenter.frames.len(), 1);
    assert_eq!(presenter.invalidations, 1);
    assert!(surface_text(&presenter.frames[0]).contains("* Working"));
    assert_eq!(
        styles_for_ascii_text(&presenter.frames[0], "* Working")[0].attributes,
        crate::surface::Attributes::BOLD
    );
}

fn run_zero_geometry_interval(first: Size, repeated: Size) {
    let period = Duration::from_secs(1);
    let mut retained = active_motion_session(period);
    let mut agent = SimpleAgent::default();
    let polls = Rc::new(Cell::new(0));
    let reads = Rc::new(Cell::new(0));
    let visible_at = Instant::now() + Duration::from_millis(40);
    let events = DelayedEventSequence::new(
        [
            Event::Resize(first.width, first.height),
            Event::Resize(repeated.width, repeated.height),
            Event::Paste("hidden-update".to_owned()),
        ],
        (visible_at, Event::Resize(16, 6)),
        Rc::clone(&polls),
        Rc::clone(&reads),
    );
    let render_count = Rc::new(Cell::new(0));
    let watchdog_fired = Rc::new(Cell::new(false));
    let started = representable_past(period + Duration::from_millis(100));
    let presenter = run_generation_with_termination_at(
        &mut retained,
        &mut agent,
        events,
        StopAfterOrWatchdog {
            counter: Rc::clone(&render_count),
            threshold: 2,
            deadline: Instant::now() + Duration::from_secs(1),
            armed: false,
            watchdog_fired: Rc::clone(&watchdog_fired),
        },
        render_count,
        started,
    );

    assert!(!watchdog_fired.get());
    assert_eq!(reads.get(), 4);
    assert!(
        polls.get() > reads.get(),
        "zero geometry never entered the pending waker path"
    );
    assert!(
        polls.get() <= 8,
        "zero geometry polled {} times",
        polls.get()
    );
    assert_eq!(presenter.frames.len(), 2);
    assert_eq!(presenter.invalidations, 3);
    assert!(!surface_text(&presenter.frames[0]).contains("hidden-update"));
    assert!(surface_text(&presenter.frames[1]).contains("hidden-update"));
    assert_eq!(
        styles_for_ascii_text(&presenter.frames[1], "* Working")[0].attributes,
        crate::surface::Attributes::BOLD
    );
}

// width가 0인 interval에서 반복 resize와 semantic 갱신이 와도 frame deadline을
// 남기지 않으며, 다시 보이면 최신 상태와 기존 generation epoch로 frame 하나만 그립니다.
#[test]
fn zero_width_interval_suppresses_busy_frames_until_one_visible_recovery() {
    run_zero_geometry_interval(Size::new(0, 7), Size::new(0, 8));
}

// height가 0인 interval도 width=0과 같은 억제 경계를 사용해 terminal poll을 폭주시킬 수
// 없고, 복구 resize가 숨은 동안의 요청을 잃지 않은 최신 frame 하나를 즉시 만듭니다.
#[test]
fn zero_height_interval_suppresses_busy_frames_until_one_visible_recovery() {
    run_zero_geometry_interval(Size::new(15, 0), Size::new(16, 0));
}

// 같은 semantic turn을 재진입해도 terminal ownership generation마다 motion epoch는
// 새로 시작하므로 고정 marker를 유지하면서 style phase만 첫 위치에서 시작한다.
#[test]
fn each_terminal_generation_starts_with_a_fresh_motion_epoch() {
    let period = Duration::from_millis(100);
    let mut retained = active_motion_session(period);
    let mut agent = SimpleAgent::default();
    let old_epoch = representable_past(Duration::from_millis(1_100));

    let first_polls = Rc::new(Cell::new(0));
    let first = run_generation_at(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&first_polls), Rc::new(Cell::new(0))),
        1,
        old_epoch,
    );
    let second_polls = Rc::new(Cell::new(0));
    let second = run_generation(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&second_polls), Rc::new(Cell::new(0))),
        1,
    );

    let first_styles = styles_for_ascii_text(&first.frames[0], "* Working");
    let second_styles = styles_for_ascii_text(&second.frames[0], "* Working");

    assert_eq!(first_styles[0].attributes, crate::surface::Attributes::BOLD);
    assert_eq!(second_styles[0].attributes, crate::surface::Attributes::DIM);
}

// Monotonic epoch가 아직 offset을 표현하지 못하면 helper는 raw unwrap panic 대신
// bounded retry를 수행하고, representable sample 또는 명명된 prerequisite error를 냅니다.
#[test]
fn motion_history_helper_retries_unrepresentable_instants_with_a_bound() {
    let expected = Instant::now();
    let mut attempts = 0;
    let resolved = retry_representable_past(Duration::from_millis(20), || {
        attempts += 1;
        (attempts == 3).then_some(expected)
    })
    .unwrap();
    assert_eq!(resolved, expected);
    assert_eq!(attempts, 3);

    let started = Instant::now();
    let error = retry_representable_past::<Instant>(Duration::from_millis(2), || None).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(error.contains("monotonic clock"));
}
