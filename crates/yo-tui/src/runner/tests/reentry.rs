use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    convert::Infallible,
    io,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

use crossterm::event::{
    Event, KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent, KeyModifiers,
};
use yo_core::{
    AgentBackend, AgentCommand, AgentEvent, AgentIntent, AgentSession, AgentSessionError,
    AgentSessionPoll, BackendCapabilities, BackendCommandEvidence, BackendFailure, BackendPoll,
    BackendStopHandle, CommandAdmission, TranscriptRecord, TurnRef,
};

use crate::{
    appearance::{AppearanceCandidate, ColorCapability, GlyphProfile, MotionPreference},
    runner::{
        AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch,
        TerminationEvent, TerminationSource, TuiSession,
        unix::{FrameViewport, LivePresenter, LoopError, LoopExit, drive},
    },
    surface::{CellContent, Point, Size, Surface},
    terminal::{
        backend::{
            ScreenModeBackend, TerminalBackend, TerminalOutputBackend,
            unix::{EventSource, UnixEventReader},
        },
        mode::{
            TerminalSession,
            screen::{ScreenMode, enter_screen},
        },
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
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
    timeouts: Rc<RefCell<Vec<Duration>>>,
    sleep_budget: usize,
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
            timeouts: Rc::new(RefCell::new(Vec::new())),
            sleep_budget: 0,
        }
    }

    fn timed(
        events: impl IntoIterator<Item = Event>,
        polls: Rc<Cell<usize>>,
        timeouts: Rc<RefCell<Vec<Duration>>>,
        sleep_budget: usize,
    ) -> Self {
        Self {
            events: events.into_iter().collect(),
            polls,
            reads: Rc::new(Cell::new(0)),
            timeouts,
            sleep_budget,
        }
    }
}

impl EventSource for Events {
    type Error = io::Error;

    fn poll(&mut self, timeout: Duration) -> Result<bool, Self::Error> {
        self.timeouts.borrow_mut().push(timeout);
        self.polls.set(self.polls.get() + 1);
        if !timeout.is_zero() && self.sleep_budget > 0 {
            self.sleep_budget -= 1;
            std::thread::sleep(timeout);
        }
        Ok(!self.events.is_empty())
    }

    fn read(&mut self) -> Result<Event, Self::Error> {
        self.reads.set(self.reads.get() + 1);
        self.events
            .pop_front()
            .ok_or_else(|| io::Error::other("no event"))
    }
}

struct StopAfter {
    counter: Rc<Cell<usize>>,
    threshold: usize,
}

impl TerminationSource for StopAfter {
    fn poll_termination(&mut self) -> TerminationEvent {
        if self.counter.get() >= self.threshold {
            TerminationEvent::Requested
        } else {
            TerminationEvent::None
        }
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
}

#[derive(Default)]
struct Presenter {
    previous_on_render: Vec<bool>,
    frames: Vec<Surface>,
}

impl FrameViewport for Presenter {
    fn invalidate_frame(&mut self) {}
}

impl LivePresenter<Backend> for Presenter {
    fn render(
        &mut self,
        _session: &mut TerminalSession<'_, Backend>,
        previous: Option<&Surface>,
        current: &Surface,
        _cursor: Point,
    ) -> Result<(), LoopError> {
        self.previous_on_render.push(previous.is_some());
        self.frames.push(current.clone());
        Ok(())
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
                std::thread::yield_now();
            },
            other => panic!("worker did not publish the expected change: {other:?}"),
        }
    }
}

fn run_generation(
    retained: &mut TuiSession,
    agent: &mut impl AgentConnection,
    events: Events,
    termination: StopAfter,
) -> Presenter {
    run_generation_at(retained, agent, events, termination, Instant::now())
}

fn run_generation_at(
    retained: &mut TuiSession,
    agent: &mut impl AgentConnection,
    events: Events,
    termination: StopAfter,
    started: Instant,
) -> Presenter {
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let mut presenter = Presenter::default();
    let mut reader = UnixEventReader::new(events, termination);

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            retained,
            agent,
            Size::new(16, 6),
            started,
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();
    presenter
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

impl AgentBackend for BlockingAgentBackend {
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
        StopAfter {
            counter: first_polls,
            threshold: 4,
        },
    );

    let second_polls = Rc::new(Cell::new(0));
    let second = run_generation(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&second_polls), Rc::new(Cell::new(0))),
        StopAfter {
            counter: second_polls,
            threshold: 2,
        },
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

    assert_eq!(
        agent
            .dispatch(AgentIntent::submit("block".to_owned()).unwrap())
            .unwrap(),
        CommandAdmission::Queued
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
        StopAfter {
            counter: polls,
            threshold: 2,
        },
    );

    assert_eq!(agent.retries, 2);
    let parts = retained.parts_mut();
    assert!(parts.pending_control.is_none());
    assert!(parts.pending_dispatch.is_none());
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
    let timeouts = Rc::new(RefCell::new(Vec::new()));
    let presenter = run_generation(
        &mut retained,
        &mut agent,
        Events::timed(
            [Event::Paste("x".to_owned())],
            Rc::clone(&polls),
            Rc::clone(&timeouts),
            1,
        ),
        StopAfter {
            counter: polls,
            threshold: 2,
        },
    );

    assert_eq!(presenter.frames.len(), 2);
    let waits = timeouts.borrow();
    assert!(!waits.is_empty());
    assert!(waits[0] > Duration::ZERO);
    assert!(waits[0] <= period);
}

// 전송 재시도 중에도 10ms backpressure poll만 반복하지 않고, 더 가까운 motion
// 마감에 맞춰 깨어나 실제 marker frame을 갱신한다.
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
    assert_eq!(
        core.dispatch(AgentIntent::submit("block".to_owned()).unwrap())
            .unwrap(),
        CommandAdmission::Queued
    );
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
    let timeouts = Rc::new(RefCell::new(Vec::new()));
    let presenter = run_generation(
        &mut retained,
        &mut agent,
        Events::timed([], Rc::clone(&polls), Rc::clone(&timeouts), 2),
        StopAfter {
            counter: polls,
            threshold: 3,
        },
    );

    release_tx.send(()).unwrap();
    wait_for_session_change(&mut core);
    core.shutdown().unwrap();

    assert!(agent.retries >= 3);
    assert!(presenter.frames.len() >= 2);
    assert!(surface_text(&presenter.frames[0]).contains("* Working"));
    assert!(
        timeouts
            .borrow()
            .iter()
            .all(|timeout| *timeout <= Duration::from_millis(10))
    );
    assert!(
        timeouts
            .borrow()
            .iter()
            .any(|timeout| *timeout < Duration::from_millis(10))
    );
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
    let termination = StopAfter {
        counter: polls,
        threshold: 2,
    };
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let mut presenter = Presenter::default();
    let mut reader = UnixEventReader::new(events, termination);
    let started = Instant::now()
        .checked_sub(period + Duration::from_millis(100))
        .unwrap();

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            &mut retained,
            &mut agent,
            Size::new(0, 0),
            started,
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();

    assert_eq!(presenter.frames.len(), 1);
    assert!(surface_text(&presenter.frames[0]).contains("* Working"));
    assert_eq!(
        styles_for_ascii_text(&presenter.frames[0], "* Working")[0].attributes,
        crate::surface::Attributes::BOLD
    );
}

// 같은 semantic turn을 재진입해도 terminal ownership generation마다 motion epoch는
// 새로 시작하므로 고정 marker를 유지하면서 style phase만 첫 위치에서 시작한다.
#[test]
fn each_terminal_generation_starts_with_a_fresh_motion_epoch() {
    let period = Duration::from_millis(100);
    let mut retained = active_motion_session(period);
    let mut agent = SimpleAgent::default();
    let old_epoch = Instant::now()
        .checked_sub(Duration::from_millis(1_100))
        .unwrap();

    let first_polls = Rc::new(Cell::new(0));
    let first = run_generation_at(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&first_polls), Rc::new(Cell::new(0))),
        StopAfter {
            counter: first_polls,
            threshold: 1,
        },
        old_epoch,
    );
    let second_polls = Rc::new(Cell::new(0));
    let second = run_generation(
        &mut retained,
        &mut agent,
        Events::new([], Rc::clone(&second_polls), Rc::new(Cell::new(0))),
        StopAfter {
            counter: second_polls,
            threshold: 1,
        },
    );

    let first_styles = styles_for_ascii_text(&first.frames[0], "* Working");
    let second_styles = styles_for_ascii_text(&second.frames[0], "* Working");

    assert_eq!(first_styles[0].attributes, crate::surface::Attributes::BOLD);
    assert_eq!(second_styles[0].attributes, crate::surface::Attributes::DIM);
}
