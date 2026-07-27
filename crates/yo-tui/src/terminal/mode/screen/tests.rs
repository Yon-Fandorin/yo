use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};

use super::{
    ScreenMode, close_inline, enter_screen, render_fullscreen, render_inline,
    run_fullscreen_boundary, run_inline_boundary,
};
use crate::{
    surface::{Point, Size, Surface},
    terminal::{
        backend::{ScreenModeBackend, TerminalBackend, TerminalOutputBackend},
        mode::{
            fullscreen::{FullscreenRenderError, FullscreenViewport},
            inline::{InlineRenderError, InlineRestoreOutcome, InlineViewport},
            panic_route::PANIC_ROUTE_TEST_OWNER,
        },
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
    EnterAlternateScreen,
    OperationPanic,
    ClearViewport,
    LeaveAlternateScreen,
    RestoreTty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClearBehavior {
    Succeed,
    Fail,
    Panic,
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
                ClearBehavior::Panic => panic!("viewport clear panic"),
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

    fn acquire_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        match mode {
            Mode::AlternateScreen => self.events.borrow_mut().push(Event::EnterAlternateScreen),
        }
        Ok(())
    }

    fn release_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        match mode {
            Mode::AlternateScreen => self.events.borrow_mut().push(Event::LeaveAlternateScreen),
        }
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

// active Fullscreen session의 writer만 빌려 frame과 shell cursor를 출력한다.
#[test]
fn fullscreen_renderer_writes_through_the_active_session() {
    let mut backend = backend(ClearBehavior::Succeed, false);
    let mut session = enter_screen(&mut backend, ScreenMode::Fullscreen).unwrap();
    let current = Surface::new(Size::new(1, 1)).unwrap();
    let mut viewport = FullscreenViewport::default();

    render_fullscreen(
        &mut session,
        &mut viewport,
        None,
        &current,
        Point::new(0, 0),
    )
    .unwrap();

    assert!(session.close().is_ok());
}

// Fullscreen renderer는 alternate screen을 소유하지 않은 Inline session에 출력하지 않는다.
#[test]
fn fullscreen_renderer_rejects_an_inline_session() {
    let mut backend = backend(ClearBehavior::Succeed, false);
    let mut session = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let current = Surface::new(Size::new(1, 1)).unwrap();
    let mut viewport = FullscreenViewport::default();

    let error = render_fullscreen(
        &mut session,
        &mut viewport,
        None,
        &current,
        Point::new(0, 0),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        FullscreenRenderError::AlternateScreenNotOwned
    ));
    assert!(session.close().is_ok());
}

// Inline renderer는 alternate screen을 소유한 Fullscreen session의 화면을 수정하지 않는다.
#[test]
fn inline_renderer_rejects_a_fullscreen_session() {
    let mut backend = backend(ClearBehavior::Succeed, false);
    let mut session = enter_screen(&mut backend, ScreenMode::Fullscreen).unwrap();
    let current = Surface::new(Size::new(1, 1)).unwrap();
    let mut viewport = InlineViewport::default();

    let error = render_inline(&mut session, &mut viewport, None, &current).unwrap_err();

    assert!(matches!(error, InlineRenderError::AlternateScreenOwned));
    assert!(session.close().is_ok());
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

// viewport clear panic과 TTY 복구 오류가 함께 발생해도 둘 다 보존한다.
#[test]
fn viewport_panic_and_terminal_failure_are_both_retained() {
    let mut backend = backend(ClearBehavior::Panic, true);
    let events = Rc::clone(&backend.events);
    let mut viewport = InlineViewport::default();
    let session = rendered_session(&mut backend, &mut viewport);

    let report = close_inline(session, &mut viewport);

    assert!(matches!(
        report.viewport,
        Err(crate::terminal::mode::transaction::CleanupFailureCause::Panicked(
            ref message
        )) if message == "viewport clear panic"
    ));
    let terminal = report.terminal.unwrap_err();
    assert_eq!(terminal.failures.len(), 1);
    assert_eq!(
        terminal.failures[0].cause,
        crate::terminal::mode::transaction::CleanupFailureCause::Error("restore tty")
    );
    assert!(events.borrow().contains(&Event::RestoreTty));
}

// 앱 panic을 주원인으로 보존하면서 viewport와 TTY를 정리한 뒤 진단을 반환한다.
#[test]
fn inline_boundary_cleans_up_before_returning_the_primary_panic() {
    let _panic_route_owner = lock_panic_route_test();
    let mut backend = backend(ClearBehavior::Succeed, false);
    let events = Rc::clone(&backend.events);
    let mut viewport = InlineViewport::default();
    let session = rendered_session(&mut backend, &mut viewport);
    let operation_events = Rc::clone(&events);

    let routed = run_inline_boundary(session, &mut viewport, move |_, _| -> () {
        operation_events.borrow_mut().push(Event::OperationPanic);
        panic!("application panic");
    })
    .unwrap();

    let report = routed.result.unwrap();
    let Err(primary) = report.operation else {
        panic!("application panic must remain the primary result");
    };
    assert_eq!(primary.downcast_ref::<&str>(), Some(&"application panic"));
    assert!(matches!(
        report.cleanup.viewport,
        Ok(InlineRestoreOutcome::Cleared)
    ));
    assert!(report.cleanup.terminal.is_ok());
    assert_eq!(routed.diagnostic.unwrap().message, "application panic");
    assert_eq!(
        events.borrow().as_slice(),
        &[
            Event::CaptureTty,
            Event::EnableRaw,
            Event::OperationPanic,
            Event::ClearViewport,
            Event::RestoreTty,
        ]
    );
}

// 앱 panic과 viewport·TTY 정리 실패가 동시에 발생해도 어느 하나도 잃지 않는다.
#[test]
fn inline_boundary_retains_primary_panic_and_both_cleanup_failures() {
    let _panic_route_owner = lock_panic_route_test();
    let mut backend = backend(ClearBehavior::Panic, true);
    let mut viewport = InlineViewport::default();
    let session = rendered_session(&mut backend, &mut viewport);

    let routed = run_inline_boundary(session, &mut viewport, |_, _| -> () {
        panic!("application panic");
    })
    .unwrap();

    let report = routed.result.unwrap();
    let Err(primary) = report.operation else {
        panic!("application panic must remain the primary result");
    };
    assert_eq!(primary.downcast_ref::<&str>(), Some(&"application panic"));
    assert!(matches!(
        report.cleanup.viewport,
        Err(crate::terminal::mode::transaction::CleanupFailureCause::Panicked(
            ref message
        )) if message == "viewport clear panic"
    ));
    assert!(report.cleanup.terminal.is_err());
    assert_eq!(routed.diagnostic.unwrap().message, "application panic");
}

// Fullscreen 앱 panic 뒤 alternate screen과 TTY를 순서대로 복구하고 원인을 보존한다.
#[test]
fn fullscreen_boundary_restores_terminal_before_returning_the_primary_panic() {
    let _panic_route_owner = lock_panic_route_test();
    let mut backend = backend(ClearBehavior::Succeed, false);
    let events = Rc::clone(&backend.events);
    let session = enter_screen(&mut backend, ScreenMode::Fullscreen).unwrap();
    let operation_events = Rc::clone(&events);

    let routed = run_fullscreen_boundary(session, move |_| -> () {
        operation_events.borrow_mut().push(Event::OperationPanic);
        panic!("fullscreen application panic");
    })
    .unwrap();

    let report = routed.result.unwrap();
    let Err(primary) = report.operation else {
        panic!("application panic must remain the primary result");
    };
    assert_eq!(
        primary.downcast_ref::<&str>(),
        Some(&"fullscreen application panic")
    );
    assert!(report.cleanup.is_ok());
    assert_eq!(
        routed.diagnostic.unwrap().message,
        "fullscreen application panic"
    );
    assert_eq!(
        events.borrow().as_slice(),
        &[
            Event::CaptureTty,
            Event::EnableRaw,
            Event::EnterAlternateScreen,
            Event::OperationPanic,
            Event::LeaveAlternateScreen,
            Event::RestoreTty,
        ]
    );
}

// Fullscreen 앱 panic과 TTY 복구 오류가 함께 발생해도 둘 다 보존한다.
#[test]
fn fullscreen_boundary_retains_primary_and_terminal_cleanup_failure() {
    let _panic_route_owner = lock_panic_route_test();
    let mut backend = backend(ClearBehavior::Succeed, true);
    let session = enter_screen(&mut backend, ScreenMode::Fullscreen).unwrap();

    let routed = run_fullscreen_boundary(session, |_| -> () {
        panic!("fullscreen application panic");
    })
    .unwrap();

    let report = routed.result.unwrap();
    let Err(primary) = report.operation else {
        panic!("application panic must remain the primary result");
    };
    assert_eq!(
        primary.downcast_ref::<&str>(),
        Some(&"fullscreen application panic")
    );
    let cleanup = report.cleanup.unwrap_err();
    assert_eq!(cleanup.failures.len(), 1);
    assert_eq!(
        cleanup.failures[0].cause,
        crate::terminal::mode::transaction::CleanupFailureCause::Error("restore tty")
    );
    assert_eq!(
        routed.diagnostic.unwrap().message,
        "fullscreen application panic"
    );
}

fn lock_panic_route_test() -> std::sync::MutexGuard<'static, ()> {
    PANIC_ROUTE_TEST_OWNER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
