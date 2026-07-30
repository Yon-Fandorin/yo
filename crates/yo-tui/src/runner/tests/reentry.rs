use std::{
    cell::Cell,
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
    AgentSessionPoll, BackendCapabilities, BackendFailure, BackendPoll, BackendStopHandle,
    CommandAdmission, TranscriptRecord, TurnRef,
};

use crate::{
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

    fn poll(&mut self, _timeout: Duration) -> Result<bool, Self::Error> {
        self.polls.set(self.polls.get() + 1);
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

impl AgentConnection for SimpleAgent {
    type Error = Infallible;

    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        if let AgentIntent::Submit(input) = action {
            self.records.push_back(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: TurnRef::new(
                        yo_core::SessionId::new(std::num::NonZeroU64::MIN),
                        yo_core::TurnId::new(std::num::NonZeroU64::MIN),
                    ),
                    input: input.into(),
                },
            ));
        }
        Ok(DispatchOutcome::Accepted)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Accepted)
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
    frames: Vec<String>,
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
        self.frames.push(surface_text(current));
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

fn key(code: CrosstermKeyCode) -> Event {
    Event::Key(CrosstermKeyEvent::new(code, KeyModifiers::NONE))
}

fn run_generation(
    retained: &mut TuiSession,
    agent: &mut impl AgentConnection,
    events: Events,
    termination: StopAfter,
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
            Instant::now(),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();
    presenter
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

    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure> {
        self.processed.send(()).unwrap();
        if matches!(command, AgentCommand::StartTurn { .. }) && !self.blocked {
            self.blocked = true;
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        Ok(())
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
    let mut retained = TuiSession::new();
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
    assert!(second.frames[0].contains("❯ question"));
    assert!(second.frames[0].contains("draft"));
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

    assert_eq!(
        agent
            .dispatch(AgentIntent::Submit("block".to_owned()))
            .unwrap(),
        CommandAdmission::Accepted
    );
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    processed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let CommandAdmission::Backpressured(normal) = agent
        .dispatch(AgentIntent::Submit("normal-pending".to_owned()))
        .unwrap()
    else {
        panic!("the full normal lane must retain the next command");
    };
    let CommandAdmission::Backpressured(control) = agent
        .dispatch(AgentIntent::Submit("control-slot-pending".to_owned()))
        .unwrap()
    else {
        panic!("the full normal lane must retain another command");
    };

    let mut retained = TuiSession::new();
    {
        let parts = retained.parts_mut();
        *parts.pending_dispatch = Some(normal);
        *parts.pending_control = Some(control);
    }

    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match agent.session.poll().unwrap() {
            AgentSessionPoll::Changed => {
                if transcript.read_after(None).entries().iter().any(|entry| {
                    matches!(
                        entry.record(),
                        TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { .. })
                    )
                }) {
                    break;
                }
            },
            AgentSessionPoll::Pending if Instant::now() < deadline => std::thread::yield_now(),
            other => {
                assert!(
                    Instant::now() < deadline,
                    "worker did not finish the blocked command: {other:?}"
                );
            },
        }
    }
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
