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
}

impl<Writer: Write> FullscreenRenderer<Writer> {
    pub(crate) const fn new(writer: Writer) -> Self {
        Self {
            ansi: AnsiEncoder::new(writer),
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
        self.ansi.encode(&operations)?;
        self.ansi.encode_operations(&[TerminalOp::MoveTo(cursor)])?;
        self.ansi
            .writer_mut()
            .flush()
            .map_err(FullscreenRenderError::Flush)?;
        pending.commit();
        Ok(())
    }

    pub(crate) fn into_inner(self) -> Writer {
        self.ansi.into_inner()
    }
}
