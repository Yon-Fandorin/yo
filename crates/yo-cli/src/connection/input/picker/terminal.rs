use std::{
    fs::File,
    io::{Read, Write},
    num::NonZeroU16,
    thread,
    time::{Duration, Instant},
};

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::termios::{self, InputFlags, LocalFlags, SetArg, SpecialCharacterIndices, Termios},
};

use super::{PickerChoice, PickerIdentity, PickerState, render_lines};
use crate::{AppError, connection::presentation::PresentationStyle};

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
    let original = OFlag::from_bits_truncate(
        fcntl(terminal, FcntlArg::F_GETFL)
            .map_err(|error| AppError::single("reading terminal picker flags", error))?,
    );
    fcntl(terminal, FcntlArg::F_SETFL(original | OFlag::O_NONBLOCK))
        .map_err(|error| AppError::single("setting terminal picker flags", error))?;
    let result = read_optional_byte_until(deadline, |byte| terminal.read(byte), Instant::now);
    let restore = fcntl(terminal, FcntlArg::F_SETFL(original))
        .map(|_| ())
        .map_err(|error| AppError::single("restoring terminal picker flags", error));
    restore?;
    result
}

fn read_optional_byte_until(
    deadline: Instant,
    mut read: impl FnMut(&mut [u8; 1]) -> std::io::Result<usize>,
    mut now: impl FnMut() -> Instant,
) -> Result<Option<u8>, AppError> {
    loop {
        let mut byte = [0_u8; 1];
        match read(&mut byte) {
            Ok(1) => return Ok(Some(byte[0])),
            Ok(0) => {
                if now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            },
            Ok(_) => unreachable!("one-byte terminal read cannot return more than one byte"),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            },
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                if now() >= deadline {
                    return Ok(None);
                }
            },
            Err(error) => {
                return Err(AppError::single("reading a terminal escape key", error));
            },
        }
    }
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

    fn raw_terminal() -> (File, File, PickerTerminalScope) {
        let pty = openpty(None, None).unwrap();
        let terminal = File::from(pty.slave);
        let master = File::from(pty.master);
        let scope = PickerTerminalScope::enter(&terminal).unwrap();
        (terminal, master, scope)
    }

    fn decode_once(bytes: &[u8]) -> PickerKey {
        let (terminal, master, _scope) = raw_terminal();
        write(&master, bytes).unwrap();
        PickerInput::new(&terminal).read_key().unwrap()
    }

    // 연속 EINTR가 발생해도 기존 sequence deadline을 세 번째 재시도에서 관찰해
    // 추가 read로 넘어가지 않고 partial key를 유한하게 종료합니다.
    #[test]
    fn repeated_interrupted_reads_stop_at_the_existing_deadline() {
        let started = Instant::now();
        let deadline = started + ESCAPE_SEQUENCE_WAIT;
        let attempts = Cell::new(0);
        let observations = Cell::new(0);

        let byte = read_optional_byte_until(
            deadline,
            |_| {
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                if attempt <= 3 {
                    Err(std::io::ErrorKind::Interrupted.into())
                } else {
                    Err(std::io::Error::other("read retried after its deadline"))
                }
            },
            || {
                let observation = observations.get() + 1;
                observations.set(observation);
                if observation < 3 { started } else { deadline }
            },
        )
        .unwrap();

        assert_eq!(byte, None);
        assert_eq!(attempts.get(), 3);
        assert_eq!(observations.get(), 3);
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
