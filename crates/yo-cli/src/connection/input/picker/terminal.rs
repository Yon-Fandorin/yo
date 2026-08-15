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

pub(super) enum PickerKey {
    Up,
    Down,
    Backspace,
    Text(String),
    Enter,
    Cancel,
    Ignore,
}

pub(super) fn read_key(terminal: &File) -> Result<PickerKey, AppError> {
    let first = read_byte(terminal)?;
    match first {
        3 => Ok(PickerKey::Cancel),
        b'\r' | b'\n' => Ok(PickerKey::Enter),
        8 | 127 => Ok(PickerKey::Backspace),
        27 => read_escape_key(terminal),
        0..=31 => Ok(PickerKey::Ignore),
        32..=126 => Ok(PickerKey::Text(char::from(first).to_string())),
        _ => read_utf8_key(terminal, first),
    }
}

fn read_byte(mut terminal: &File) -> Result<u8, AppError> {
    let mut byte = [0_u8; 1];
    terminal
        .read_exact(&mut byte)
        .map_err(|error| AppError::single("reading the model picker", error))?;
    Ok(byte[0])
}

fn read_escape_key(terminal: &File) -> Result<PickerKey, AppError> {
    let Some(second) = read_optional_byte(terminal)? else {
        return Ok(PickerKey::Cancel);
    };
    if second != b'[' {
        return Ok(PickerKey::Cancel);
    }
    match read_optional_byte(terminal)? {
        Some(b'A') => Ok(PickerKey::Up),
        Some(b'B') => Ok(PickerKey::Down),
        _ => Ok(PickerKey::Ignore),
    }
}

fn read_optional_byte(mut terminal: &File) -> Result<Option<u8>, AppError> {
    let original = OFlag::from_bits_truncate(
        fcntl(terminal, FcntlArg::F_GETFL)
            .map_err(|error| AppError::single("reading terminal picker flags", error))?,
    );
    fcntl(terminal, FcntlArg::F_SETFL(original | OFlag::O_NONBLOCK))
        .map_err(|error| AppError::single("setting terminal picker flags", error))?;
    let deadline = Instant::now() + ESCAPE_SEQUENCE_WAIT;
    let result = loop {
        let mut byte = [0_u8; 1];
        match terminal.read(&mut byte) {
            Ok(1) => break Ok(Some(byte[0])),
            Ok(_) => break Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            },
            Err(error) => break Err(AppError::single("reading a terminal escape key", error)),
        }
    };
    let restore = fcntl(terminal, FcntlArg::F_SETFL(original))
        .map(|_| ())
        .map_err(|error| AppError::single("restoring terminal picker flags", error));
    restore?;
    result
}

fn read_utf8_key(terminal: &File, first: u8) -> Result<PickerKey, AppError> {
    let length = match first {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return Ok(PickerKey::Ignore),
    };
    let mut bytes = vec![first; length];
    let mut reader = terminal;
    reader
        .read_exact(&mut bytes[1..])
        .map_err(|error| AppError::single("reading UTF-8 model search input", error))?;
    match String::from_utf8(bytes) {
        Ok(value) => Ok(PickerKey::Text(value)),
        Err(_) => Ok(PickerKey::Ignore),
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
