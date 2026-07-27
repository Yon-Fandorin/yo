use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use super::{InlineFrameError, InlineFramePlan, InlineRestorePlan, PendingFrame, PendingRestore};
use crate::{
    surface::{Point, Surface},
    terminal::{AnsiEncoder, TerminalOp, TerminalOps},
};

#[derive(Debug)]
pub(crate) enum InlineRenderError {
    AlternateScreenOwned,
    Frame(InlineFrameError),
    Output(io::Error),
}

impl fmt::Display for InlineRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlternateScreenOwned => {
                formatter.write_str("inline rendering requires the main screen")
            },
            Self::Frame(error) => write!(formatter, "inline frame is inconsistent: {error}"),
            Self::Output(_) => formatter.write_str("writing the inline viewport failed"),
        }
    }
}

impl Error for InlineRenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AlternateScreenOwned => None,
            Self::Frame(error) => Some(error),
            Self::Output(error) => Some(error),
        }
    }
}

impl From<InlineFrameError> for InlineRenderError {
    fn from(error: InlineFrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<io::Error> for InlineRenderError {
    fn from(error: io::Error) -> Self {
        Self::Output(error)
    }
}

pub(crate) struct InlineRenderer<Writer> {
    ansi: AnsiEncoder<Writer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineRestoreOutcome {
    Nothing,
    Cleared,
    LeftUntrusted { abandoned_rows: u16 },
}

impl<Writer: Write> InlineRenderer<Writer> {
    pub(crate) const fn new(writer: Writer) -> Self {
        Self {
            ansi: AnsiEncoder::new(writer),
        }
    }

    pub(crate) fn render(
        &mut self,
        pending: PendingFrame<'_>,
        previous: Option<&Surface>,
        current: &Surface,
    ) -> Result<(), InlineRenderError> {
        let plan = pending.plan();
        let operations = TerminalOps::from_diff(&pending.diff(previous, current)?);
        let mut cursor = Cursor::prepare(self.ansi.writer_mut(), plan)?;

        for operation in operations.as_slice() {
            match *operation {
                TerminalOp::FrameSizeChanged { .. } => {},
                TerminalOp::MoveTo(point) => {
                    cursor.move_to(self.ansi.writer_mut(), point)?;
                },
                content => self.ansi.encode_content_operation(content)?,
            }
        }

        cursor.clear_surplus(self.ansi.writer_mut(), plan)?;
        cursor.move_to(self.ansi.writer_mut(), Point::new(0, current.size().height))?;
        self.ansi.writer_mut().flush()?;
        pending.commit();
        Ok(())
    }

    pub(crate) fn restore(
        &mut self,
        pending: PendingRestore<'_>,
    ) -> Result<InlineRestoreOutcome, InlineRenderError> {
        let outcome = match pending.plan() {
            InlineRestorePlan::Nothing => InlineRestoreOutcome::Nothing,
            InlineRestorePlan::LeaveUntrusted { abandoned_rows } => {
                InlineRestoreOutcome::LeftUntrusted { abandoned_rows }
            },
            InlineRestorePlan::ClearOwned { size } => {
                let mut cursor = Cursor::from_anchor(self.ansi.writer_mut(), size.height)?;
                for row in 0..size.height {
                    cursor.move_to(self.ansi.writer_mut(), Point::new(0, row))?;
                    self.ansi.writer_mut().write_all(b"\x1b[2K")?;
                }
                cursor.move_to(self.ansi.writer_mut(), Point::new(0, 0))?;
                self.ansi.writer_mut().flush()?;
                InlineRestoreOutcome::Cleared
            },
        };
        pending.commit();
        Ok(outcome)
    }

    pub(crate) fn into_inner(self) -> Writer {
        self.ansi.into_inner()
    }
}

#[derive(Clone, Copy, Debug)]
struct Cursor {
    row: u16,
}

impl Cursor {
    fn prepare(writer: &mut impl Write, plan: InlineFramePlan) -> io::Result<Self> {
        let anchor_distance = match plan {
            InlineFramePlan::Initialize { current } => {
                allocate_rows(writer, current.height)?;
                current.height
            },
            InlineFramePlan::Update { current } => current.height,
            InlineFramePlan::Reconcile {
                previous, current, ..
            } => {
                if current.height > previous.height {
                    allocate_rows(writer, current.height - previous.height)?;
                    current.height
                } else {
                    previous.height
                }
            },
            InlineFramePlan::Reanchor {
                abandoned_rows,
                current,
            } => {
                allocate_rows(writer, abandoned_rows)?;
                allocate_rows(writer, current.height)?;
                current.height
            },
        };

        move_up(writer, anchor_distance)?;
        move_to_column(writer, 0)?;
        Ok(Self { row: 0 })
    }

    fn from_anchor(writer: &mut impl Write, viewport_height: u16) -> io::Result<Self> {
        move_up(writer, viewport_height)?;
        move_to_column(writer, 0)?;
        Ok(Self { row: 0 })
    }

    fn move_to(&mut self, writer: &mut impl Write, point: Point) -> io::Result<()> {
        if point.y < self.row {
            move_up(writer, self.row - point.y)?;
        } else {
            move_down(writer, point.y - self.row)?;
        }
        move_to_column(writer, point.x)?;
        self.row = point.y;
        Ok(())
    }

    fn clear_surplus(&mut self, writer: &mut impl Write, plan: InlineFramePlan) -> io::Result<()> {
        let InlineFramePlan::Reconcile {
            previous, current, ..
        } = plan
        else {
            return Ok(());
        };

        for row in current.height..previous.height {
            self.move_to(writer, Point::new(0, row))?;
            writer.write_all(b"\x1b[2K")?;
        }
        Ok(())
    }
}

fn allocate_rows(writer: &mut impl Write, rows: u16) -> io::Result<()> {
    if rows == 0 {
        return Ok(());
    }
    writer.write_all(b"\r")?;
    for _ in 0..rows {
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn move_up(writer: &mut impl Write, rows: u16) -> io::Result<()> {
    if rows > 0 {
        write!(writer, "\x1b[{rows}A")?;
    }
    Ok(())
}

fn move_down(writer: &mut impl Write, rows: u16) -> io::Result<()> {
    if rows > 0 {
        write!(writer, "\x1b[{rows}B")?;
    }
    Ok(())
}

fn move_to_column(writer: &mut impl Write, column: u16) -> io::Result<()> {
    write!(writer, "\x1b[{}G", u32::from(column) + 1)
}

#[cfg(test)]
mod tests;
