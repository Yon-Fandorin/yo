use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use super::{FullscreenFrameError, PendingFullscreenFrame};
use crate::{
    surface::Surface,
    terminal::{
        AnsiEncodeError, AnsiEncoder, RESET_HYPERLINK, TerminalOp, TerminalOps, graphics::Graphics,
    },
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
    graphics: Graphics,
    hyperlink_dirty: bool,
}

impl<Writer: Write> FullscreenRenderer<Writer> {
    pub(crate) fn new(writer: Writer) -> Self {
        Self {
            ansi: AnsiEncoder::new(writer),
            frame_bytes: Vec::new(),
            graphics: Graphics::from_env(),
            hyperlink_dirty: false,
        }
    }

    pub(crate) fn render(
        &mut self,
        pending: PendingFullscreenFrame<'_>,
        previous: Option<&Surface>,
        current: &Surface,
    ) -> Result<(), FullscreenRenderError> {
        if self.hyperlink_dirty {
            self.ansi
                .writer_mut()
                .write_all(RESET_HYPERLINK)
                .map_err(|error| FullscreenRenderError::Ansi(AnsiEncodeError::Io(error)))?;
        }
        self.hyperlink_dirty = false;
        let result = self.render_frame(pending, previous, current);
        if result.is_ok() {
            self.hyperlink_dirty = false;
        } else if self.hyperlink_dirty {
            let _ = self.ansi.writer_mut().write_all(RESET_HYPERLINK);
            let _ = self.ansi.writer_mut().write_all(END_UPDATE);
            let _ = self.ansi.writer_mut().flush();
        }
        result
    }

    fn render_frame(
        &mut self,
        pending: PendingFullscreenFrame<'_>,
        previous: Option<&Surface>,
        current: &Surface,
    ) -> Result<(), FullscreenRenderError> {
        let cursor = pending.cursor();
        let diff = pending.diff(previous, current)?;
        let operations = TerminalOps::from_diff(&diff);
        self.frame_bytes.clear();
        self.frame_bytes.extend_from_slice(BEGIN_UPDATE);
        let mut encoder = AnsiEncoder::new(&mut self.frame_bytes);
        encoder.encode(&operations)?;
        if previous.is_none() {
            self.graphics.clear(&mut self.frame_bytes);
        }
        self.graphics
            .prepare(previous, current, &diff, &mut self.frame_bytes);
        let mut encoder = AnsiEncoder::new(&mut self.frame_bytes);
        encoder.encode_operations(&[TerminalOp::MoveTo(cursor)])?;
        self.frame_bytes.extend_from_slice(END_UPDATE);
        // Batch writes alone do not prevent the terminal from displaying a
        // partially updated word. Include the diff and final cursor in one
        // synchronized update; unsupported terminals may ignore the mode.
        self.hyperlink_dirty = current.has_hyperlinks();
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

#[cfg(test)]
mod graphics_tests {
    use super::*;
    use crate::{
        surface::{Point, RasterImage, Rect, Size},
        terminal::mode::fullscreen::FullscreenViewport,
    };

    // resize/recovery가 이전 Surface를 버려도 마지막 native placement는 삭제되어야 한다.
    #[test]
    fn discarded_previous_surface_still_clears_native_placements() {
        let size = Size::new(20, 10);
        let mut image_frame = Surface::new(size).unwrap();
        image_frame.rasters.push(RasterImage {
            area: Rect::new(Point::new(1, 1), Size::new(5, 3)),
            png: vec![1, 2, 3].into(),
        });
        let mut viewport = FullscreenViewport::default();
        let mut renderer = FullscreenRenderer::new(Vec::new());
        renderer.graphics = Graphics::Kitty;
        renderer
            .render(
                viewport.begin_frame(size, Point::new(0, 9)).unwrap(),
                None,
                &image_frame,
            )
            .unwrap();
        renderer.ansi.writer_mut().clear();
        let resized = Surface::new(Size::new(21, 10)).unwrap();
        renderer
            .render(
                viewport
                    .begin_frame(resized.size(), Point::new(0, 9))
                    .unwrap(),
                None,
                &resized,
            )
            .unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert!(output.contains("a=d,d=I,i="));
        assert!(!output.contains("a=T"));
    }
}
