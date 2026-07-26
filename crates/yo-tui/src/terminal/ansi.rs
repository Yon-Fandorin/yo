use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use super::{TerminalOp, TerminalOps};
use crate::surface::{Attributes, Color, Size, Style};

/// Encodes validated terminal operations into deterministic ANSI bytes.
pub struct AnsiEncoder<Writer> {
    writer: Writer,
}

impl<Writer: Write> AnsiEncoder<Writer> {
    #[must_use]
    pub const fn new(writer: Writer) -> Self {
        Self { writer }
    }

    pub fn encode(&mut self, operations: &TerminalOps<'_>) -> Result<(), AnsiEncodeError> {
        self.encode_operations(operations.as_slice())
    }

    #[must_use]
    pub fn into_inner(self) -> Writer {
        self.writer
    }

    pub(crate) fn encode_operations(
        &mut self,
        operations: &[TerminalOp<'_>],
    ) -> Result<(), AnsiEncodeError> {
        if let Some((previous, current)) = operations.iter().find_map(|operation| {
            if let TerminalOp::FrameSizeChanged { previous, current } = operation {
                Some((*previous, *current))
            } else {
                None
            }
        }) {
            return Err(AnsiEncodeError::FrameSizeChanged { previous, current });
        }

        for operation in operations {
            self.encode_operation(*operation)?;
        }
        Ok(())
    }

    fn encode_operation(&mut self, operation: TerminalOp<'_>) -> io::Result<()> {
        match operation {
            TerminalOp::FrameSizeChanged { .. } => {
                unreachable!("frame-size changes are rejected before writing bytes")
            },
            TerminalOp::MoveTo(point) => {
                write!(
                    self.writer,
                    "\x1b[{};{}H",
                    u32::from(point.y) + 1,
                    u32::from(point.x) + 1
                )
            },
            TerminalOp::SetStyle(style) => self.encode_style(style),
            TerminalOp::WriteGrapheme { text, .. } => self.writer.write_all(text.as_bytes()),
            TerminalOp::WriteBlank { count } => self.encode_blanks(count.get()),
        }
    }

    fn encode_style(&mut self, style: Style) -> io::Result<()> {
        self.writer.write_all(b"\x1b[0")?;
        self.encode_color(style.foreground, true)?;
        self.encode_color(style.background, false)?;

        for (attribute, parameter) in [
            (Attributes::BOLD, 1),
            (Attributes::DIM, 2),
            (Attributes::ITALIC, 3),
            (Attributes::UNDERLINE, 4),
            (Attributes::BLINK, 5),
            (Attributes::REVERSE, 7),
            (Attributes::HIDDEN, 8),
            (Attributes::STRIKETHROUGH, 9),
        ] {
            if style.attributes.contains(attribute) {
                write!(self.writer, ";{parameter}")?;
            }
        }

        self.writer.write_all(b"m")
    }

    fn encode_color(&mut self, color: Color, foreground: bool) -> io::Result<()> {
        let default = if foreground { 39 } else { 49 };
        let extended = if foreground { 38 } else { 48 };
        match color {
            Color::Default => write!(self.writer, ";{default}"),
            Color::Indexed(index) => write!(self.writer, ";{extended};5;{index}"),
            Color::Rgb { red, green, blue } => {
                write!(self.writer, ";{extended};2;{red};{green};{blue}")
            },
        }
    }

    fn encode_blanks(&mut self, count: u16) -> io::Result<()> {
        const BLANKS: [u8; 256] = [b' '; 256];
        let mut remaining = usize::from(count);
        while remaining > 0 {
            let chunk = remaining.min(BLANKS.len());
            self.writer.write_all(&BLANKS[..chunk])?;
            remaining -= chunk;
        }
        Ok(())
    }
}

/// Why terminal operations could not be encoded.
#[derive(Debug)]
pub enum AnsiEncodeError {
    FrameSizeChanged { previous: Size, current: Size },
    Io(io::Error),
}

impl fmt::Display for AnsiEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameSizeChanged { previous, current } => write!(
                formatter,
                "frame size changed from {}x{} to {}x{}; the mode controller must reconcile its owned region",
                previous.width, previous.height, current.width, current.height
            ),
            Self::Io(_) => formatter.write_str("writing ANSI output failed"),
        }
    }
}

impl Error for AnsiEncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FrameSizeChanged { .. } => None,
            Self::Io(error) => Some(error),
        }
    }
}

impl From<io::Error> for AnsiEncodeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
