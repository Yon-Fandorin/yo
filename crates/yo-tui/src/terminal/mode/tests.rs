use std::collections::VecDeque;

use super::transaction::{CleanupFailureCause, CleanupStep, TerminalSession};
use crate::terminal::backend::TerminalBackend;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Paste,
    AlternateScreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    CaptureTty,
    EnableRawInput,
    Acquire(Mode),
    Release(Mode),
    RestoreTty,
}

#[derive(Debug, Eq, PartialEq)]
struct Error(&'static str);

#[derive(Default)]
struct RecordingBackend {
    calls: Vec<Call>,
    failures: VecDeque<(Call, Error)>,
    panics: VecDeque<Call>,
}

impl RecordingBackend {
    fn failing(call: Call, error: &'static str) -> Self {
        Self {
            failures: VecDeque::from([(call, Error(error))]),
            ..Self::default()
        }
    }

    fn record(&mut self, call: Call) -> Result<(), Error> {
        self.calls.push(call);
        if self.panics.front() == Some(&call) {
            self.panics.pop_front();
            panic!("recording backend panic");
        }
        if self
            .failures
            .front()
            .is_some_and(|(failed, _)| *failed == call)
        {
            return Err(self.failures.pop_front().unwrap().1);
        }
        Ok(())
    }
}

impl TerminalBackend for RecordingBackend {
    type TtyState = ();
    type Mode = Mode;
    type Error = Error;

    fn capture_tty_state(&mut self) -> Result<Self::TtyState, Self::Error> {
        self.record(Call::CaptureTty)
    }

    fn enable_raw_input(&mut self, _original: &Self::TtyState) -> Result<(), Self::Error> {
        self.record(Call::EnableRawInput)
    }

    fn acquire_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        self.record(Call::Acquire(mode))
    }

    fn release_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        self.record(Call::Release(mode))
    }

    fn restore_tty_state(&mut self, _state: &Self::TtyState) -> Result<(), Self::Error> {
        self.record(Call::RestoreTty)
    }
}

// 모든 변경 전에 TTY를 저장하고, 정상 종료에서는 mode를 역순으로 해제한 뒤 TTY를 복구한다.
#[test]
fn normal_close_restores_modes_in_reverse_order_and_tty_last() {
    let mut backend = RecordingBackend::default();
    let session =
        TerminalSession::enter(&mut backend, [Mode::Paste, Mode::AlternateScreen]).unwrap();

    session.close().unwrap();

    assert_eq!(
        backend.calls,
        [
            Call::CaptureTty,
            Call::EnableRawInput,
            Call::Acquire(Mode::Paste),
            Call::Acquire(Mode::AlternateScreen),
            Call::Release(Mode::AlternateScreen),
            Call::Release(Mode::Paste),
            Call::RestoreTty,
        ]
    );
}

// raw input 적용 결과가 불확실하게 실패해도 미리 등록한 원래 TTY 복구를 시도한다.
#[test]
fn raw_input_failure_restores_the_captured_tty_state() {
    let mut backend = RecordingBackend::failing(Call::EnableRawInput, "raw input");

    let failure = match TerminalSession::enter(&mut backend, []) {
        Ok(_) => panic!("raw input failure must reject entry"),
        Err(failure) => failure,
    };

    assert_eq!(failure.primary, Error("raw input"));
    assert!(failure.cleanup.is_empty());
    assert_eq!(
        backend.calls,
        [Call::CaptureTty, Call::EnableRawInput, Call::RestoreTty]
    );
}

// mode 적용이 일부 진행된 뒤 실패해도 그 mode 자체를 포함해 등록된 보상을 모두 실행한다.
#[test]
fn partial_mode_acquisition_rolls_back_the_uncertain_mode_too() {
    let mut backend =
        RecordingBackend::failing(Call::Acquire(Mode::AlternateScreen), "alternate screen");

    let failure = match TerminalSession::enter(&mut backend, [Mode::Paste, Mode::AlternateScreen]) {
        Ok(_) => panic!("partial mode acquisition must reject entry"),
        Err(failure) => failure,
    };

    assert_eq!(failure.primary, Error("alternate screen"));
    assert!(failure.cleanup.is_empty());
    assert_eq!(
        backend.calls,
        [
            Call::CaptureTty,
            Call::EnableRawInput,
            Call::Acquire(Mode::Paste),
            Call::Acquire(Mode::AlternateScreen),
            Call::Release(Mode::AlternateScreen),
            Call::Release(Mode::Paste),
            Call::RestoreTty,
        ]
    );
}

// cleanup 하나가 실패해도 나머지를 계속 시도하고, 원래 실패와 cleanup 실패를 함께 보존한다.
#[test]
fn primary_failure_is_not_masked_by_cleanup_failures() {
    let mut backend = RecordingBackend {
        failures: VecDeque::from([
            (Call::Release(Mode::AlternateScreen), Error("leave screen")),
            (Call::RestoreTty, Error("restore tty")),
        ]),
        ..RecordingBackend::default()
    };
    let session =
        TerminalSession::enter(&mut backend, [Mode::Paste, Mode::AlternateScreen]).unwrap();

    let failure = session.finish_with_error(Error("render"));

    assert_eq!(failure.primary, Error("render"));
    assert_eq!(
        failure
            .cleanup
            .iter()
            .map(|failure| failure.step)
            .collect::<Vec<_>>(),
        [
            CleanupStep::Mode(Mode::AlternateScreen),
            CleanupStep::TtyState
        ]
    );
    assert_eq!(
        failure
            .cleanup
            .iter()
            .map(|failure| &failure.cause)
            .collect::<Vec<_>>(),
        [
            &CleanupFailureCause::Error(Error("leave screen")),
            &CleanupFailureCause::Error(Error("restore tty")),
        ]
    );
    assert_eq!(
        &backend.calls[backend.calls.len() - 3..],
        [
            Call::Release(Mode::AlternateScreen),
            Call::Release(Mode::Paste),
            Call::RestoreTty,
        ],
        "앞선 cleanup 실패와 무관하게 나머지 mode와 TTY 복구를 모두 시도해야 한다"
    );
}

// 명시적 cleanup이 panic해도 이를 구조화하고 남은 mode와 TTY 복구를 계속 시도한다.
#[test]
fn explicit_cleanup_panic_is_reported_without_skipping_later_steps() {
    let mut backend = RecordingBackend {
        panics: VecDeque::from([Call::Release(Mode::AlternateScreen)]),
        ..RecordingBackend::default()
    };
    let session =
        TerminalSession::enter(&mut backend, [Mode::Paste, Mode::AlternateScreen]).unwrap();

    let failure = session.close().unwrap_err();

    assert_eq!(failure.failures.len(), 1);
    assert_eq!(
        failure.failures[0].step,
        CleanupStep::Mode(Mode::AlternateScreen)
    );
    assert_eq!(
        failure.failures[0].cause,
        CleanupFailureCause::Panicked("recording backend panic".to_owned())
    );
    assert_eq!(
        backend.calls,
        [
            Call::CaptureTty,
            Call::EnableRawInput,
            Call::Acquire(Mode::Paste),
            Call::Acquire(Mode::AlternateScreen),
            Call::Release(Mode::AlternateScreen),
            Call::Release(Mode::Paste),
            Call::RestoreTty,
        ]
    );
}

// 명시적 복구를 마친 session의 Drop은 같은 cleanup을 다시 실행하지 않는다.
#[test]
fn explicit_close_makes_drop_idempotent() {
    let mut backend = RecordingBackend::default();
    let session = TerminalSession::enter(&mut backend, [Mode::Paste]).unwrap();

    session.close().unwrap();

    assert_eq!(
        backend
            .calls
            .iter()
            .filter(|call| **call == Call::Release(Mode::Paste))
            .count(),
        1
    );
    assert_eq!(
        backend
            .calls
            .iter()
            .filter(|call| **call == Call::RestoreTty)
            .count(),
        1
    );
}

// Drop fallback은 backend cleanup panic을 밖으로 전파하지 않고 다음 복구도 계속 시도한다.
#[test]
fn drop_is_non_panicking_and_continues_after_backend_panic() {
    let mut backend = RecordingBackend {
        panics: VecDeque::from([Call::Release(Mode::Paste)]),
        ..RecordingBackend::default()
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _session =
            TerminalSession::enter(&mut backend, [Mode::Paste, Mode::AlternateScreen]).unwrap();
    }));

    assert!(result.is_ok());
    assert_eq!(
        backend.calls.last(),
        Some(&Call::RestoreTty),
        "cleanup panic 뒤에도 최종 TTY 복구를 시도해야 한다"
    );
}
