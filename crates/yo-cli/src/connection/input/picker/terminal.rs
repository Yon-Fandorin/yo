use std::{
    fs::File,
    io::{Read, Write},
    num::NonZeroU16,
    os::fd::{AsRawFd, RawFd},
    time::{Duration, Instant},
};

use nix::sys::termios::{self, InputFlags, LocalFlags, SetArg, SpecialCharacterIndices, Termios};

use super::{PickerChoice, PickerIdentity, PickerState, render_lines};
use crate::{AppError, presentation::PresentationStyle};

const ESCAPE_SEQUENCE_WAIT: Duration = Duration::from_millis(25);
const MAX_ESCAPE_SEQUENCE_BYTES: usize = 32;

pub(super) struct PickerTerminalScope {
    terminal: File,
    original: Option<Termios>,
    rendered_rows: usize,
}

impl PickerTerminalScope {
    pub(super) fn enter(terminal: &File) -> Result<Self, AppError> {
        let terminal = terminal
            .try_clone()
            .map_err(|error| AppError::single("cloning the controlling terminal", error))?;
        let original = termios::tcgetattr(&terminal)
            .map_err(|error| AppError::single("reading terminal picker settings", error))?;
        let mut raw = original.clone();
        raw.input_flags.remove(
            InputFlags::BRKINT
                | InputFlags::ICRNL
                | InputFlags::INPCK
                | InputFlags::ISTRIP
                | InputFlags::IXON,
        );
        raw.local_flags
            .remove(LocalFlags::ECHO | LocalFlags::ICANON | LocalFlags::IEXTEN | LocalFlags::ISIG);
        raw.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
        raw.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
        termios::tcsetattr(&terminal, SetArg::TCSAFLUSH, &raw)
            .map_err(|error| AppError::single("entering terminal picker mode", error))?;
        let mut scope = Self {
            terminal,
            original: Some(original),
            rendered_rows: 0,
        };
        scope
            .terminal
            .write_all(b"\x1b[?25l")
            .and_then(|()| scope.terminal.flush())
            .map_err(|error| AppError::single("starting the model picker", error))?;
        Ok(scope)
    }

    pub(super) fn render(
        &mut self,
        identity: &PickerIdentity,
        state: &PickerState,
        choices: &[PickerChoice],
        style: PresentationStyle,
    ) -> Result<(), AppError> {
        let width = terminal_width(&self.terminal);
        self.clear_panel()
            .map_err(|error| AppError::single("clearing the model picker", error))?;
        let lines = render_lines(identity, state, choices, width, style);
        let mut output = lines.join("\n");
        output.push('\n');
        self.rendered_rows = lines.len();
        self.terminal
            .write_all(output.as_bytes())
            .and_then(|()| self.terminal.flush())
            .map_err(|error| AppError::single("rendering the model picker", error))?;
        Ok(())
    }

    fn clear_panel(&mut self) -> std::io::Result<()> {
        if self.rendered_rows > 0 {
            write!(self.terminal, "\x1b[{}A\r\x1b[J", self.rendered_rows)?;
            self.rendered_rows = 0;
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<(), AppError> {
        let display_result = self
            .clear_panel()
            .and_then(|()| self.terminal.write_all(b"\x1b[?25h"))
            .and_then(|()| self.terminal.flush())
            .map_err(|error| AppError::single("cleaning up the model picker", error));
        let restore_result = self.restore();
        display_result?;
        restore_result
    }

    fn restore(&mut self) -> Result<(), AppError> {
        let Some(original) = self.original.take() else {
            return Ok(());
        };
        termios::tcsetattr(&self.terminal, SetArg::TCSAFLUSH, &original)
            .map_err(|error| AppError::single("restoring terminal picker settings", error))
    }
}

impl Drop for PickerTerminalScope {
    fn drop(&mut self) {
        let _ = self.clear_panel();
        let _ = self.terminal.write_all(b"\x1b[?25h");
        let _ = self.terminal.flush();
        if let Some(original) = self.original.take() {
            let _ = termios::tcsetattr(&self.terminal, SetArg::TCSAFLUSH, &original);
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum PickerKey {
    Up,
    Down,
    Backspace,
    Text(String),
    Enter,
    Cancel,
    Ignore,
}

pub(super) struct PickerInput<'a> {
    terminal: &'a File,
    pending: Option<u8>,
    discarding_escape_tail: bool,
}

impl<'a> PickerInput<'a> {
    pub(super) const fn new(terminal: &'a File) -> Self {
        Self {
            terminal,
            pending: None,
            discarding_escape_tail: false,
        }
    }

    pub(super) fn read_key(&mut self) -> Result<PickerKey, AppError> {
        if self.discarding_escape_tail {
            return self.discard_escape_tail(Instant::now() + ESCAPE_SEQUENCE_WAIT);
        }
        let first = self.read_byte()?;
        match first {
            3 => Ok(PickerKey::Cancel),
            b'\r' | b'\n' => Ok(PickerKey::Enter),
            8 | 127 => Ok(PickerKey::Backspace),
            27 => self.read_escape_key(),
            0..=31 => Ok(PickerKey::Ignore),
            32..=126 => Ok(PickerKey::Text(char::from(first).to_string())),
            _ => self.read_utf8_key(first),
        }
    }

    fn read_byte(&mut self) -> Result<u8, AppError> {
        if let Some(byte) = self.pending.take() {
            return Ok(byte);
        }
        let mut terminal = self.terminal;
        let mut byte = [0_u8; 1];
        terminal
            .read_exact(&mut byte)
            .map_err(|error| AppError::single("reading the model picker", error))?;
        Ok(byte[0])
    }

    fn read_escape_key(&mut self) -> Result<PickerKey, AppError> {
        let deadline = Instant::now() + ESCAPE_SEQUENCE_WAIT;
        let Some(second) = read_optional_byte(self.terminal, deadline)? else {
            return Ok(PickerKey::Cancel);
        };
        match second {
            b'[' | b'O' => self.read_control_sequence(deadline),
            _ => Ok(PickerKey::Ignore),
        }
    }

    fn read_control_sequence(&mut self, deadline: Instant) -> Result<PickerKey, AppError> {
        let mut parameters: Vec<u8> = Vec::new();
        for _ in 0..MAX_ESCAPE_SEQUENCE_BYTES {
            let Some(byte) = read_optional_byte(self.terminal, deadline)? else {
                return Ok(PickerKey::Ignore);
            };
            if (0x40..=0x7e).contains(&byte) {
                let arrow_parameters = parameters
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':'));
                return Ok(match (byte, arrow_parameters) {
                    (b'A', true) => PickerKey::Up,
                    (b'B', true) => PickerKey::Down,
                    _ => PickerKey::Ignore,
                });
            }
            if !(0x20..=0x3f).contains(&byte) {
                self.discarding_escape_tail = true;
                return Ok(PickerKey::Ignore);
            }
            parameters.push(byte);
        }
        self.discarding_escape_tail = true;
        Ok(PickerKey::Ignore)
    }

    fn discard_escape_tail(&mut self, deadline: Instant) -> Result<PickerKey, AppError> {
        for _ in 0..MAX_ESCAPE_SEQUENCE_BYTES {
            let Some(byte) = read_optional_byte(self.terminal, deadline)? else {
                self.discarding_escape_tail = false;
                return Ok(PickerKey::Ignore);
            };
            if (0x40..=0x7e).contains(&byte) {
                self.discarding_escape_tail = false;
                return Ok(PickerKey::Ignore);
            }
        }
        Ok(PickerKey::Ignore)
    }

    fn read_utf8_key(&mut self, first: u8) -> Result<PickerKey, AppError> {
        let length = match first {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return Ok(PickerKey::Ignore),
        };
        let deadline = Instant::now() + ESCAPE_SEQUENCE_WAIT;
        let mut bytes = Vec::with_capacity(length);
        bytes.push(first);
        for index in 1..length {
            let Some(byte) = read_optional_byte(self.terminal, deadline)? else {
                return Ok(PickerKey::Ignore);
            };
            if !valid_utf8_continuation(first, index, byte) {
                if !(0x80..=0xbf).contains(&byte) {
                    self.pending = Some(byte);
                }
                return Ok(PickerKey::Ignore);
            }
            bytes.push(byte);
        }
        match String::from_utf8(bytes) {
            Ok(value) => Ok(PickerKey::Text(value)),
            Err(_) => Ok(PickerKey::Ignore),
        }
    }
}

fn read_optional_byte(mut terminal: &File, deadline: Instant) -> Result<Option<u8>, AppError> {
    read_optional_byte_until(
        deadline,
        |remaining| wait_for_terminal_input(terminal.as_raw_fd(), remaining),
        |byte| terminal.read(byte),
        Instant::now,
    )
}

fn read_optional_byte_until(
    deadline: Instant,
    mut wait: impl FnMut(Duration) -> std::io::Result<bool>,
    mut read: impl FnMut(&mut [u8; 1]) -> std::io::Result<usize>,
    mut now: impl FnMut() -> Instant,
) -> Result<Option<u8>, AppError> {
    loop {
        let observed = now();
        if observed >= deadline {
            return Ok(None);
        }
        match wait(deadline.saturating_duration_since(observed)) {
            Ok(true) => {},
            Ok(false) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(AppError::single("waiting for a terminal escape key", error));
            },
        }
        if now() >= deadline {
            return Ok(None);
        }
        let mut byte = [0_u8; 1];
        match read(&mut byte) {
            Ok(1) => return Ok(Some(byte[0])),
            Ok(0) => return Ok(None),
            Ok(_) => unreachable!("one-byte terminal read cannot return more than one byte"),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
            Err(error) => {
                return Err(AppError::single("reading a terminal escape key", error));
            },
        }
    }
}

// A readiness deadline is portable across Linux and Darwin PTYs, while toggling
// `O_NONBLOCK` on a terminal with `VMIN = 1` does not provide the same guarantee.
// The crate does not enable nix's poll feature, so this keeps the raw boundary
// local instead of widening the dependency feature surface for one descriptor.
#[allow(unsafe_code)]
fn wait_for_terminal_input(terminal: RawFd, timeout: Duration) -> std::io::Result<bool> {
    let timeout_millis = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
    let mut descriptor = libc::pollfd {
        fd: terminal,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `terminal` remains borrowed by the caller for this poll, and the
    // pointer names exactly one initialized pollfd for the duration of the call.
    let result = unsafe { libc::poll(&mut descriptor, 1, timeout_millis) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if result == 0 {
        return Ok(false);
    }
    if descriptor.revents & libc::POLLNVAL != 0 {
        return Err(std::io::Error::other(
            "terminal descriptor became invalid while waiting for input",
        ));
    }
    if descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
        return Ok(true);
    }
    if descriptor.revents & libc::POLLERR != 0 {
        return Err(std::io::Error::other(
            "terminal reported an input readiness error",
        ));
    }
    Ok(false)
}

fn valid_utf8_continuation(first: u8, index: usize, byte: u8) -> bool {
    if index > 1 {
        return (0x80..=0xbf).contains(&byte);
    }
    match first {
        0xC2..=0xDF | 0xE1..=0xEC | 0xEE..=0xEF | 0xF1..=0xF3 => (0x80..=0xbf).contains(&byte),
        0xE0 => (0xA0..=0xBF).contains(&byte),
        0xED => (0x80..=0x9F).contains(&byte),
        0xF0 => (0x90..=0xBF).contains(&byte),
        0xF4 => (0x80..=0x8F).contains(&byte),
        _ => false,
    }
}

fn terminal_width(terminal: &File) -> usize {
    rustix::termios::tcgetwinsize(terminal)
        .ok()
        .and_then(|size| NonZeroU16::new(size.ws_col))
        .unwrap_or_else(super::default_width)
        .get()
        .into()
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs::File, thread, time::Instant};

    use nix::{pty::openpty, unistd::write};

    use super::*;

    const TEST_TERMINAL_OUTPUT_WAIT: Duration = Duration::from_secs(1);

    struct RawTerminalScope {
        scope: Option<PickerTerminalScope>,
        output: Option<File>,
        output_wait: Duration,
    }

    impl Drop for RawTerminalScope {
        fn drop(&mut self) {
            let mut output = self
                .output
                .take()
                .expect("test terminal output descriptor must remain owned");
            let output_wait = self.output_wait;
            // Start the finite drain immediately before restoration so time
            // spent in the test body cannot consume the cleanup deadline.
            let output_drainer =
                thread::spawn(move || read_terminal_lifecycle_output(&mut output, output_wait));
            drop(self.scope.take());
            let output = output_drainer.join();
            if thread::panicking() {
                return;
            }
            assert_eq!(
                output
                    .expect("test terminal output drainer must not panic")
                    .expect("test terminal must publish lifecycle output"),
                *b"\x1b[?25l\x1b[?25h"
            );
        }
    }

    fn raw_terminal() -> (File, File, RawTerminalScope) {
        let pty = openpty(None, None).unwrap();
        let terminal = File::from(pty.slave);
        let master = File::from(pty.master);
        let scope = PickerTerminalScope::enter(&terminal).unwrap();
        let output = master.try_clone().unwrap();
        (
            terminal,
            master,
            RawTerminalScope {
                scope: Some(scope),
                output: Some(output),
                output_wait: TEST_TERMINAL_OUTPUT_WAIT,
            },
        )
    }

    fn read_terminal_lifecycle_output(
        output: &mut File,
        timeout: Duration,
    ) -> std::io::Result<[u8; 12]> {
        let deadline = Instant::now() + timeout;
        let mut lifecycle_output = [0_u8; 12];
        let mut filled = 0;
        while filled < lifecycle_output.len() {
            let observed = Instant::now();
            if observed >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "test terminal published {filled} of {} lifecycle bytes before its deadline",
                        lifecycle_output.len()
                    ),
                ));
            }
            match wait_for_terminal_input(
                output.as_raw_fd(),
                deadline.saturating_duration_since(observed),
            ) {
                Ok(true) => {},
                Ok(false) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            match output.read(&mut lifecycle_output[filled..]) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        format!(
                            "test terminal closed after {filled} of {} lifecycle bytes",
                            lifecycle_output.len()
                        ),
                    ));
                },
                Ok(read) => filled += read,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
                Err(error) => return Err(error),
            }
        }
        Ok(lifecycle_output)
    }

    fn decode_once(bytes: &[u8]) -> PickerKey {
        let (terminal, master, _scope) = raw_terminal();
        write(&master, bytes).unwrap();
        PickerInput::new(&terminal).read_key().unwrap()
    }

    // PTY lifecycle output이 부족하면 test drainer가 무기한 join되지 않고 byte 수를
    // 포함한 timeout으로 끝나 회귀 테스트 자체의 finite 경계를 보존합니다.
    #[test]
    fn terminal_lifecycle_output_drain_has_a_finite_deadline() {
        let pty = openpty(None, None).unwrap();
        let _terminal = File::from(pty.slave);
        let mut master = File::from(pty.master);
        let started = Instant::now();

        let error = read_terminal_lifecycle_output(&mut master, ESCAPE_SEQUENCE_WAIT).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("0 of 12 lifecycle bytes"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    // test 본문이 cleanup budget보다 오래 실행돼도 output deadline은 scope Drop 직전에
    // 시작되어 cursor hide/show를 모두 drain하고 Darwin restoration을 막지 않습니다.
    #[test]
    fn terminal_lifecycle_output_deadline_starts_with_cleanup() {
        let (terminal, master, mut scope) = raw_terminal();
        scope.output_wait = ESCAPE_SEQUENCE_WAIT;
        thread::sleep(ESCAPE_SEQUENCE_WAIT + ESCAPE_SEQUENCE_WAIT);

        drop(scope);
        drop((terminal, master));
    }

    // readiness poll에 연속 EINTR가 발생해도 기존 sequence deadline을 세 번째
    // 재시도에서 관찰해 실제 read 없이 partial key를 유한하게 종료합니다.
    #[test]
    fn repeated_interrupted_reads_stop_at_the_existing_deadline() {
        let started = Instant::now();
        let deadline = started + ESCAPE_SEQUENCE_WAIT;
        let waits = Cell::new(0);
        let reads = Cell::new(0);
        let observations = Cell::new(0);

        let byte = read_optional_byte_until(
            deadline,
            |_| {
                let wait = waits.get() + 1;
                waits.set(wait);
                if wait <= 2 {
                    Err(std::io::ErrorKind::Interrupted.into())
                } else {
                    Err(std::io::Error::other("poll retried after its deadline"))
                }
            },
            |_| {
                reads.set(reads.get() + 1);
                Err(std::io::Error::other("read occurred without readiness"))
            },
            || {
                let observation = observations.get() + 1;
                observations.set(observation);
                if observation < 3 { started } else { deadline }
            },
        )
        .unwrap();

        assert_eq!(byte, None);
        assert_eq!(waits.get(), 2);
        assert_eq!(reads.get(), 0);
        assert_eq!(observations.get(), 3);
    }

    // poll이 readiness를 반환하더라도 deadline 관찰이 이미 만료라면 뒤늦은 byte를
    // 읽지 않아 escape sequence의 finite 경계를 넘지 않는지 검증합니다.
    #[test]
    fn readiness_observed_after_the_deadline_does_not_start_a_read() {
        let started = Instant::now();
        let deadline = started + ESCAPE_SEQUENCE_WAIT;
        let observations = Cell::new(0);
        let reads = Cell::new(0);

        let byte = read_optional_byte_until(
            deadline,
            |_| Ok(true),
            |_| {
                reads.set(reads.get() + 1);
                Ok(1)
            },
            || {
                let observation = observations.get() + 1;
                observations.set(observation);
                if observation == 1 { started } else { deadline }
            },
        )
        .unwrap();

        assert_eq!(byte, None);
        assert_eq!(reads.get(), 0);
        assert_eq!(observations.get(), 2);
    }

    // CSI·SS3의 plain/modified arrow만 동작하고 Delete, PageDown, F1 같은 완성된
    // unsupported sequence는 한 key로 소비되어 검색 문자열 residue를 남기지 않습니다.
    #[test]
    fn bounded_escape_decoder_maps_arrows_and_consumes_unsupported_sequences() {
        assert_eq!(decode_once(b"\x1b[A"), PickerKey::Up);
        assert_eq!(decode_once(b"\x1b[1;5B"), PickerKey::Down);
        assert_eq!(decode_once(b"\x1bOA"), PickerKey::Up);

        for sequence in [b"\x1b[3~".as_slice(), b"\x1b[6~", b"\x1bOP"] {
            let (terminal, master, _scope) = raw_terminal();
            let mut bytes = sequence.to_vec();
            bytes.push(b'z');
            write(&master, &bytes).unwrap();
            let mut input = PickerInput::new(&terminal);

            assert_eq!(input.read_key().unwrap(), PickerKey::Ignore);
            assert_eq!(input.read_key().unwrap(), PickerKey::Text("z".to_owned()));
        }
    }

    // Bare ESC는 짧은 deadline 뒤 취소로 남고, final byte 없는 truncated sequence는
    // 같은 deadline 안에 Ignore로 돌아와 picker 입력을 무기한 막지 않습니다.
    #[test]
    fn bare_escape_cancels_while_truncated_sequence_returns_within_the_deadline() {
        let started = Instant::now();
        assert_eq!(decode_once(b"\x1b"), PickerKey::Cancel);
        assert!(started.elapsed() < Duration::from_millis(100));

        let started = Instant::now();
        assert_eq!(decode_once(b"\x1b[12"), PickerKey::Ignore);
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    // Overlong 또는 malformed CSI의 tail은 bounded recovery 상태에서 final까지 버리고,
    // 그 뒤 독립 ASCII key만 다음 검색 입력으로 보존합니다.
    #[test]
    fn malformed_and_overlong_escape_tails_never_become_search_text() {
        for mut sequence in [
            {
                let mut bytes = b"\x1b[".to_vec();
                bytes.extend(std::iter::repeat_n(b'1', 40));
                bytes.push(b'~');
                bytes
            },
            b"\x1b[\x01123~".to_vec(),
        ] {
            let (terminal, master, _scope) = raw_terminal();
            sequence.push(b'z');
            write(&master, &sequence).unwrap();
            let mut input = PickerInput::new(&terminal);

            assert_eq!(input.read_key().unwrap(), PickerKey::Ignore);
            assert_eq!(input.read_key().unwrap(), PickerKey::Ignore);
            assert_eq!(input.read_key().unwrap(), PickerKey::Text("z".to_owned()));
        }
    }

    // 2·3·4-byte scalar가 PTY에 나뉘어 도착해도 전체 sequence deadline 안이면 한 Text
    // key가 되고 terminal byte 분할은 query 문자를 바꾸지 않습니다.
    #[test]
    fn split_valid_utf8_scalars_decode_as_one_key() {
        for value in ["é", "한", "😀"] {
            let (terminal, master, _scope) = raw_terminal();
            // Keep one master descriptor alive until the decoder consumes the
            // last continuation byte. Closing the only master immediately after
            // writing can turn the slave read into a hangup before queued input
            // is delivered.
            let master_keepalive = master.try_clone().unwrap();
            let bytes = value.as_bytes().to_vec();
            let writer = thread::spawn(move || {
                write(&master, &bytes[..1]).unwrap();
                thread::sleep(Duration::from_millis(5));
                write(&master, &bytes[1..]).unwrap();
            });
            let decoded = PickerInput::new(&terminal).read_key().unwrap();
            writer.join().unwrap();
            drop(master_keepalive);

            assert_eq!(decoded, PickerKey::Text(value.to_owned()));
        }
    }

    // 잘린 UTF-8 lead는 유한 시간에 Ignore가 되고, 잘못된 continuation으로 먼저 읽은
    // ASCII/ESC byte는 pending slot을 통해 다음 독립 key로 다시 해석됩니다.
    #[test]
    fn truncated_or_invalid_utf8_is_bounded_and_preserves_the_next_key() {
        let started = Instant::now();
        assert_eq!(decode_once(&[0xE2]), PickerKey::Ignore);
        assert!(started.elapsed() < Duration::from_millis(100));

        for bytes in [vec![0xE2, b'z'], vec![0xF0, 0x90, b'\x1b']] {
            let (terminal, master, _scope) = raw_terminal();
            write(&master, &bytes).unwrap();
            let mut input = PickerInput::new(&terminal);

            assert_eq!(input.read_key().unwrap(), PickerKey::Ignore);
            let expected = if bytes[bytes.len() - 1] == b'\x1b' {
                PickerKey::Cancel
            } else {
                PickerKey::Text("z".to_owned())
            };
            assert_eq!(input.read_key().unwrap(), expected);
        }

        let (terminal, master, _scope) = raw_terminal();
        write(&master, &[0xC0, b'z']).unwrap();
        let mut input = PickerInput::new(&terminal);
        assert_eq!(input.read_key().unwrap(), PickerKey::Ignore);
        assert_eq!(input.read_key().unwrap(), PickerKey::Text("z".to_owned()));
    }
}
