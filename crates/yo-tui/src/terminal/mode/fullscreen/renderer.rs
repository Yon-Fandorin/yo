use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use super::{FullscreenFrameError, PendingFullscreenFrame};
use crate::{
    surface::Surface,
    terminal::{AnsiEncodeError, AnsiEncoder, TerminalOp, TerminalOps},
};

const BEGIN_UPDATE: &[u8] = b"\x1b[?2026h";
const END_UPDATE: &[u8] = b"\x1b[?2026l";

#[derive(Debug)]
pub(crate) enum FullscreenRenderError {
    AlternateScreenNotOwned,
    Frame(FullscreenFrameError),
    Ansi(AnsiEncodeError),
    Flush(io::Error),
}

impl fmt::Display for FullscreenRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlternateScreenNotOwned => {
                formatter.write_str("fullscreen rendering requires alternate-screen ownership")
            },
            Self::Frame(error) => write!(formatter, "fullscreen frame is inconsistent: {error}"),
            Self::Ansi(error) => write!(formatter, "encoding the fullscreen frame failed: {error}"),
            Self::Flush(_) => formatter.write_str("flushing the fullscreen frame failed"),
        }
    }
}

impl Error for FullscreenRenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AlternateScreenNotOwned => None,
            Self::Frame(error) => Some(error),
            Self::Ansi(error) => Some(error),
            Self::Flush(error) => Some(error),
        }
    }
}

impl From<FullscreenFrameError> for FullscreenRenderError {
    fn from(error: FullscreenFrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<AnsiEncodeError> for FullscreenRenderError {
    fn from(error: AnsiEncodeError) -> Self {
        Self::Ansi(error)
    }
}

pub(crate) struct FullscreenRenderer<Writer> {
    ansi: AnsiEncoder<Writer>,
    frame_bytes: Vec<u8>,
}

impl<Writer: Write> FullscreenRenderer<Writer> {
    pub(crate) const fn new(writer: Writer) -> Self {
        Self {
            ansi: AnsiEncoder::new(writer),
            frame_bytes: Vec::new(),
        }
    }

    pub(crate) fn render(
        &mut self,
        pending: PendingFullscreenFrame<'_>,
        previous: Option<&Surface>,
        current: &Surface,
    ) -> Result<(), FullscreenRenderError> {
        let cursor = pending.cursor();
        let operations = TerminalOps::from_diff(&pending.diff(previous, current)?);
        self.frame_bytes.clear();
        self.frame_bytes.extend_from_slice(BEGIN_UPDATE);
        let mut encoder = AnsiEncoder::new(&mut self.frame_bytes);
        encoder.encode(&operations)?;
        encoder.encode_operations(&[TerminalOp::MoveTo(cursor)])?;
        self.frame_bytes.extend_from_slice(END_UPDATE);
        // Batch writes alone do not prevent the terminal from displaying a
        // partially updated word. Include the diff and final cursor in one
        // synchronized update; unsupported terminals may ignore the mode.
        let result = self
            .ansi
            .writer_mut()
            .write_all(&self.frame_bytes)
            .map_err(|error| FullscreenRenderError::Ansi(AnsiEncodeError::Io(error)))
            .and_then(|()| {
                self.ansi
                    .writer_mut()
                    .flush()
                    .map_err(FullscreenRenderError::Flush)
            });
        if let Err(error) = result {
            // A partial write can leave update mode enabled. Best-effort release
            // must not commit the frame or replace the original output error.
            let _ = self.ansi.writer_mut().write_all(END_UPDATE);
            let _ = self.ansi.writer_mut().flush();
            return Err(error);
        }
        pending.commit();
        Ok(())
    }

    pub(crate) fn into_inner(self) -> Writer {
        self.ansi.into_inner()
    }
}
