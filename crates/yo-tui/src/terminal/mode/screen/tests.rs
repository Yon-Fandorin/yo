use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};

use super::{ScreenMode, close_inline, enter_screen, render_inline};
use crate::{
    surface::{Size, Surface},
    terminal::{
        backend::{ScreenModeBackend, TerminalBackend, TerminalOutputBackend},
        mode::inline::{InlineRestoreOutcome, InlineViewport},
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    AlternateScreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Event {
    CaptureTty,
    EnableRaw,
    ClearViewport,
    RestoreTty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClearBehavior {
    Succeed,
    Fail,
}

struct RecordingWriter {
    events: Rc<RefCell<Vec<Event>>>,
    clear_behavior: ClearBehavior,
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes == b"\x1b[2K" {
            self.events.borrow_mut().push(Event::ClearViewport);
            match self.clear_behavior {
                ClearBehavior::Succeed => {},
                ClearBehavior::Fail => {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "clear"));
                },
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct RecordingBackend {
    events: Rc<RefCell<Vec<Event>>>,
    output: RecordingWriter,
    fail_restore: bool,
}

impl TerminalBackend for RecordingBackend {
    type TtyState = ();
    type Mode = Mode;
    type Error = &'static str;

    fn capture_tty_state(&mut self) -> Result<Self::TtyState, Self::Error> {
        self.events.borrow_mut().push(Event::CaptureTty);
        Ok(())
    }

    fn enable_raw_input(&mut self, _original: &Self::TtyState) -> Result<(), Self::Error> {
        self.events.borrow_mut().push(Event::EnableRaw);
        Ok(())
    }

    fn acquire_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
        Ok(())
    }

    fn release_mode(&mut self, _mode: Self::Mode) -> Result<(), Self::Error> {
        Ok(())
    }

    fn restore_tty_state(&mut self, _state: &Self::TtyState) -> Result<(), Self::Error> {
        self.events.borrow_mut().push(Event::RestoreTty);
        if self.fail_restore {
            Err("restore tty")
        } else {
            Ok(())
        }
    }
}

impl ScreenModeBackend for RecordingBackend {
    fn alternate_screen_mode() -> Self::Mode {
        Mode::AlternateScreen
    }
}

impl TerminalOutputBackend for RecordingBackend {
    type Output = RecordingWriter;

    fn output(&mut self) -> &mut Self::Output {
        &mut self.output
    }
}

fn backend(clear_behavior: ClearBehavior, fail_restore: bool) -> RecordingBackend {
    let events = Rc::new(RefCell::new(Vec::new()));
    RecordingBackend {
        output: RecordingWriter {
            events: Rc::clone(&events),
            clear_behavior,
        },
        events,
        fail_restore,
    }
}

fn rendered_session<'backend>(
    backend: &'backend mut RecordingBackend,
    viewport: &mut InlineViewport,
) -> crate::terminal::mode::TerminalSession<'backend, RecordingBackend> {
    let mut session = enter_screen(backend, ScreenMode::Inline).unwrap();
    let current = Surface::new(Size::new(1, 1)).unwrap();
    render_inline(&mut session, viewport, None, &current).unwrap();
    session
}

// 정상 outer close는 Inline viewport를 먼저 지운 뒤 마지막에 TTY를 복구한다.
#[test]
fn inline_cleanup_precedes_tty_restoration() {
    let mut backend = backend(ClearBehavior::Succeed, false);
    let events = Rc::clone(&backend.events);
    let mut viewport = InlineViewport::default();
    let session = rendered_session(&mut backend, &mut viewport);

    let report = close_inline(session, &mut viewport);

    assert!(matches!(report.viewport, Ok(InlineRestoreOutcome::Cleared)));
    assert!(report.terminal.is_ok());
    let events = events.borrow();
    let clear = events
        .iter()
        .position(|event| *event == Event::ClearViewport)
        .unwrap();
    let restore = events
        .iter()
        .position(|event| *event == Event::RestoreTty)
        .unwrap();
    assert!(clear < restore);
}

// viewport clear가 오류를 반환해도 TTY 복구를 계속하고 두 결과를 분리해 보존한다.
#[test]
fn viewport_error_does_not_skip_tty_restoration() {
    let mut backend = backend(ClearBehavior::Fail, false);
    let events = Rc::clone(&backend.events);
    let mut viewport = InlineViewport::default();
    let session = rendered_session(&mut backend, &mut viewport);

    let report = close_inline(session, &mut viewport);

    assert!(report.viewport.is_err());
    assert!(report.terminal.is_ok());
    assert!(events.borrow().contains(&Event::RestoreTty));
}

// presentation과 TTY 복구가 모두 실패하면 어느 한쪽도 다른 원인을 가리지 않는다.
#[test]
fn viewport_and_terminal_failures_are_both_retained() {
    let mut backend = backend(ClearBehavior::Fail, true);
    let mut viewport = InlineViewport::default();
    let session = rendered_session(&mut backend, &mut viewport);

    let report = close_inline(session, &mut viewport);

    assert!(report.viewport.is_err());
    let terminal = report.terminal.unwrap_err();
    assert_eq!(terminal.failures.len(), 1);
    assert_eq!(
        terminal.failures[0].cause,
        crate::terminal::mode::transaction::CleanupFailureCause::Error("restore tty")
    );
}
