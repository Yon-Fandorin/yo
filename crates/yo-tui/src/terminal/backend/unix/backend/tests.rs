use std::io::{self, Write};

use super::{UnixBackend, UnixBackendError, UnixMode};
use crate::terminal::{
    backend::{
        TerminalBackend,
        unix::{TermiosDriver, TtyStateAdapter},
    },
    mode::{
        TerminalSession,
        screen::{ScreenMode, enter_screen},
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct TtyState {
    raw: bool,
}

#[derive(Default)]
struct RecordingTermios {
    applied: Vec<TtyState>,
}

impl TermiosDriver for RecordingTermios {
    type State = TtyState;
    type Error = &'static str;

    fn capture(&mut self) -> Result<Self::State, Self::Error> {
        Ok(TtyState { raw: false })
    }

    fn make_raw(&mut self, state: &mut Self::State) {
        state.raw = true;
    }

    fn apply(&mut self, state: &Self::State) -> Result<(), Self::Error> {
        self.applied.push(state.clone());
        Ok(())
    }
}

#[derive(Default)]
struct RecordingWriter {
    bytes: Vec<u8>,
    flushes: usize,
    fail_flush: bool,
    fail_write_after: Option<usize>,
    write_failed: bool,
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(remaining) = self.fail_write_after
            && !self.write_failed
        {
            if remaining == 0 {
                self.write_failed = true;
                return Err(io::Error::other("write failed"));
            }
            let written = remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..written]);
            self.fail_write_after = Some(remaining - written);
            return Ok(written);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            Err(io::Error::other("flush failed"))
        } else {
            Ok(())
        }
    }
}

fn backend(writer: RecordingWriter) -> UnixBackend<RecordingTermios, RecordingWriter> {
    UnixBackend::new(TtyStateAdapter::new(RecordingTermios::default()), writer)
}

// alternate screen 진입과 해제는 yo가 소유한 정확한 ANSI bytes를 즉시 flush한다.
#[test]
fn alternate_screen_has_symmetric_owned_bytes() {
    let mut backend = backend(RecordingWriter::default());

    backend.acquire_mode(UnixMode::AlternateScreen).unwrap();
    backend.release_mode(UnixMode::AlternateScreen).unwrap();

    assert_eq!(backend.output.bytes, b"\x1b[?1049h\x1b[?1049l");
    assert_eq!(backend.output.flushes, 2);
}

// mode write의 flush 실패는 성공으로 숨기지 않아 lifecycle rollback을 실행할 수 있게 한다.
#[test]
fn uncertain_flush_failure_is_reported_as_output_failure() {
    let mut backend = backend(RecordingWriter {
        fail_flush: true,
        ..RecordingWriter::default()
    });

    let error = backend.acquire_mode(UnixMode::AlternateScreen).unwrap_err();

    match error {
        UnixBackendError::Output(error) => {
            assert_eq!(error.kind(), io::ErrorKind::Other);
        },
        UnixBackendError::Tty(_) => panic!("flush failure must remain an output error"),
    }
    assert_eq!(backend.output.bytes, b"\x1b[?1049h");
}

// 진입 flush가 불확실하게 실패하면 같은 concrete backend로 inverse mode와 TTY 복구를 수행한다.
#[test]
fn uncertain_entry_failure_uses_the_shared_transactional_rollback() {
    let mut backend = backend(RecordingWriter {
        fail_flush: true,
        ..RecordingWriter::default()
    });

    let failure = match TerminalSession::enter(&mut backend, [UnixMode::AlternateScreen]) {
        Ok(_) => panic!("uncertain alternate-screen entry must fail"),
        Err(failure) => failure,
    };

    assert!(matches!(failure.primary, UnixBackendError::Output(_)));
    assert_eq!(failure.cleanup.len(), 1);
    assert_eq!(backend.output.bytes, b"\x1b[?1049h\x1b[?1049l");
    assert_eq!(
        backend.tty.driver.applied,
        [TtyState { raw: true }, TtyState { raw: false }]
    );
}

// mode sequence가 중간까지만 쓰인 경우에도 inverse sequence와 원본 TTY 복구를 시도한다.
#[test]
fn partial_mode_write_uses_the_same_pre_registered_rollback() {
    let mut backend = backend(RecordingWriter {
        fail_write_after: Some(4),
        ..RecordingWriter::default()
    });

    let failure = match TerminalSession::enter(&mut backend, [UnixMode::AlternateScreen]) {
        Ok(_) => panic!("partial alternate-screen write must fail"),
        Err(failure) => failure,
    };

    assert!(matches!(failure.primary, UnixBackendError::Output(_)));
    assert!(failure.cleanup.is_empty());
    assert_eq!(backend.output.bytes, b"\x1b[?1\x1b[?1049l");
    assert_eq!(
        backend.tty.driver.applied,
        [TtyState { raw: true }, TtyState { raw: false }]
    );
}

// concrete backend는 저장된 TTY로 raw 상태를 만들고 동일한 원본을 복구한다.
#[test]
fn concrete_backend_connects_tty_capture_raw_entry_and_restoration() {
    let mut backend = backend(RecordingWriter::default());
    let original = backend.capture_tty_state().unwrap();

    backend.enable_raw_input(&original).unwrap();
    backend.restore_tty_state(&original).unwrap();

    assert_eq!(
        backend.tty.driver.applied,
        [TtyState { raw: true }, TtyState { raw: false }]
    );
}

// Inline recipe는 main screen을 유지하며 alternate-screen bytes를 전혀 쓰지 않는다.
#[test]
fn inline_recipe_never_acquires_the_alternate_screen() {
    let mut backend = backend(RecordingWriter::default());

    enter_screen(&mut backend, ScreenMode::Inline)
        .unwrap()
        .close()
        .unwrap();

    assert!(backend.output.bytes.is_empty());
    assert_eq!(
        backend.tty.driver.applied,
        [TtyState { raw: true }, TtyState { raw: false }]
    );
}

// Fullscreen recipe만 alternate screen을 획득하고 정상 종료에서 대칭적으로 해제한다.
#[test]
fn fullscreen_recipe_owns_the_alternate_screen() {
    let mut backend = backend(RecordingWriter::default());

    enter_screen(&mut backend, ScreenMode::Fullscreen)
        .unwrap()
        .close()
        .unwrap();

    assert_eq!(backend.output.bytes, b"\x1b[?1049h\x1b[?1049l");
}
