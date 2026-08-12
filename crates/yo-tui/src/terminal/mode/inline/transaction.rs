//! Typed, exact-byte publication transaction for the compact Inline live region.

mod ledger;
#[cfg(test)]
mod tests;

use std::{io, io::Write as _, num::NonZeroU16};

use ledger::{EffectEvidence, Ledger, ScrollEvidence};

use super::InlineFramePlan;
use crate::{
    surface::{CellContent, Point, Size, Style, Surface},
    terminal::{AnsiEncoder, TerminalOp, TerminalOps},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhysicalEffect {
    CursorVisibility,
    AddressableOwnedRows { rows: u16 },
    PublicationRow { row: u16 },
    LiveTail,
}

#[derive(Debug)]
struct EncodedOperation<'frame> {
    terminal_ops: Vec<TerminalOp<'frame>>,
    bytes: Vec<u8>,
    effect: PhysicalEffect,
    evidence: EffectEvidence,
}

#[derive(Debug)]
pub(super) struct PublicationTransaction<'frame> {
    operations: Vec<EncodedOperation<'frame>>,
    first_publication: usize,
    anchor_owned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactCorrection {
    ReversibleRestart { completed_rows: u16 },
    IrreversibleResume,
}

#[derive(Debug)]
pub(super) struct AttemptFailure {
    pub(super) error: io::Error,
    pub(super) operation: Option<usize>,
    pub(super) admitted_in_operation: usize,
}

impl<'frame> PublicationTransaction<'frame> {
    pub(super) fn compile(
        plan: InlineFramePlan,
        terminal_size: Size,
        publication: &'frame Surface,
        live_operations: &TerminalOps<'frame>,
    ) -> Self {
        let mut ledger = Ledger::new(plan, terminal_size);
        let mut operations = Vec::new();
        push(
            &mut operations,
            &mut ledger,
            vec![TerminalOp::SetCursorVisible(false)],
            PhysicalEffect::CursorVisibility,
        );
        let preparation = prepare_persistent_rows(plan);
        if !preparation.is_empty() {
            push(
                &mut operations,
                &mut ledger,
                preparation,
                PhysicalEffect::AddressableOwnedRows {
                    rows: previous_owned_rows(plan),
                },
            );
        }
        let first_publication = operations.len();
        for row in 0..publication.size().height {
            push(
                &mut operations,
                &mut ledger,
                publication_row(publication, row),
                PhysicalEffect::PublicationRow { row },
            );
        }

        // The live suffix is one self-delimiting operation. A zero-byte failure at
        // its boundary can use the publication ledger; admission inside the suffix
        // is deliberately partial-operation fatal rather than guessing which live
        // allocation, paint, caret, or visibility effect occurred.
        let mut live_tail = Vec::new();
        allocate_rows(&mut live_tail, current_size(plan).height);
        move_up(&mut live_tail, current_size(plan).height);
        move_to_column(&mut live_tail, 0);
        let mut cursor = RelativeCursor { row: 0 };
        for operation in live_operations.as_slice() {
            match *operation {
                TerminalOp::FrameSizeChanged { .. } => {},
                TerminalOp::MoveTo(point) => cursor.move_to(&mut live_tail, point),
                content => live_tail.push(content),
            }
        }
        cursor.move_to(&mut live_tail, target_cursor(plan));
        live_tail.push(TerminalOp::SetCursorVisible(true));
        push(
            &mut operations,
            &mut ledger,
            live_tail,
            PhysicalEffect::LiveTail,
        );
        Self {
            operations,
            first_publication,
            anchor_owned: matches!(
                plan,
                InlineFramePlan::Update { .. } | InlineFramePlan::Reconcile { .. }
            ),
        }
    }

    pub(super) fn execute_from(
        &self,
        writer: &mut impl io::Write,
        start: usize,
    ) -> Result<(), AttemptFailure> {
        for (index, operation) in self.operations.iter().enumerate().skip(start) {
            debug_assert!(!operation.terminal_ops.is_empty());
            let mut progress = CountingWriter::new(writer);
            if let Err(error) = progress.write_all(&operation.bytes) {
                return Err(AttemptFailure {
                    error,
                    operation: Some(index),
                    admitted_in_operation: progress.admitted,
                });
            }
        }
        writer.flush().map_err(|error| AttemptFailure {
            error,
            operation: None,
            admitted_in_operation: 0,
        })
    }

    pub(super) fn exact_correction_before(&self, operation: usize) -> Option<ExactCorrection> {
        let completed = &self.operations[..operation];
        if completed
            .iter()
            .any(|operation| operation.evidence.scroll == ScrollEvidence::Definite)
        {
            return Some(ExactCorrection::IrreversibleResume);
        }
        if completed
            .iter()
            .any(|operation| operation.evidence.scroll == ScrollEvidence::Possible)
        {
            return None;
        }

        let completed_rows = completed
            .iter()
            .filter(|operation| matches!(operation.effect, PhysicalEffect::PublicationRow { .. }))
            .count();
        let completed_rows = u16::try_from(completed_rows).ok()?;
        let mutating_prefix = completed.iter().any(|operation| {
            !matches!(
                operation.effect,
                PhysicalEffect::CursorVisibility | PhysicalEffect::PublicationRow { .. }
            )
        });
        if (self.anchor_owned || !mutating_prefix) && (self.anchor_owned || completed_rows == 0) {
            Some(ExactCorrection::ReversibleRestart { completed_rows })
        } else {
            None
        }
    }

    pub(super) fn reconcile(
        &self,
        writer: &mut impl io::Write,
        failed_operation: usize,
        correction: ExactCorrection,
    ) -> Result<(), io::Error> {
        let start = match correction {
            ExactCorrection::IrreversibleResume => failed_operation,
            ExactCorrection::ReversibleRestart { completed_rows: 0 } => failed_operation,
            ExactCorrection::ReversibleRestart { completed_rows } => {
                write_typed(writer, &clear_completed_rows(completed_rows))?;
                self.first_publication
            },
        };
        self.execute_from(writer, start)
            .map_err(|failure| failure.error)
    }
}

struct CountingWriter<'writer, Writer> {
    writer: &'writer mut Writer,
    admitted: usize,
}

impl<'writer, Writer> CountingWriter<'writer, Writer> {
    fn new(writer: &'writer mut Writer) -> Self {
        Self {
            writer,
            admitted: 0,
        }
    }
}

impl<Writer: io::Write> io::Write for CountingWriter<'_, Writer> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(bytes)?;
        self.admitted = self
            .admitted
            .checked_add(written)
            .ok_or_else(|| io::Error::other("terminal operation progress overflowed"))?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[derive(Clone, Copy, Debug)]
struct RelativeCursor {
    row: u16,
}

impl RelativeCursor {
    fn move_to<'frame>(&mut self, output: &mut Vec<TerminalOp<'frame>>, point: Point) {
        if point.y < self.row {
            move_up(output, self.row - point.y);
        } else {
            move_down(output, point.y - self.row);
        }
        move_to_column(output, point.x);
        self.row = point.y;
    }
}

fn prepare_persistent_rows(plan: InlineFramePlan) -> Vec<TerminalOp<'static>> {
    let mut output = Vec::new();
    match plan {
        InlineFramePlan::Initialize { .. } => {},
        InlineFramePlan::Update {
            current,
            previous_cursor,
            ..
        } => clear_owned_rows(&mut output, current.height, previous_cursor),
        InlineFramePlan::Reconcile {
            previous,
            previous_cursor,
            ..
        } => clear_owned_rows(&mut output, previous.height, previous_cursor),
        InlineFramePlan::Reanchor { abandoned_rows, .. } => {
            allocate_rows(&mut output, abandoned_rows);
        },
    }
    output
}

fn clear_owned_rows(output: &mut Vec<TerminalOp<'static>>, height: u16, cursor: Point) {
    move_down(output, height - cursor.y);
    move_up(output, height);
    move_to_column(output, 0);
    let mut tracked = RelativeCursor { row: 0 };
    for row in 0..height {
        tracked.move_to(output, Point::new(0, row));
        output.push(TerminalOp::EraseLine);
    }
    tracked.move_to(output, Point::new(0, 0));
}

fn publication_row(surface: &'_ Surface, row: u16) -> Vec<TerminalOp<'_>> {
    let mut output = vec![TerminalOp::CarriageReturn];
    let width = surface.size().width;
    let end = (0..width)
        .rev()
        .find(|column| {
            !matches!(
                surface
                    .cell(Point::new(*column, row))
                    .expect("the publication row stays inside its Surface")
                    .content(),
                CellContent::Blank
            )
        })
        .map_or(0, |column| column + 1);
    let mut column = 0;
    let mut style = None;
    while column < end {
        let cell = surface
            .cell(Point::new(column, row))
            .expect("the publication row stays inside its Surface");
        if style != Some(cell.style()) {
            output.push(TerminalOp::SetStyle(cell.style()));
            style = Some(cell.style());
        }
        match cell.content() {
            CellContent::Blank => {
                let count = (column..end)
                    .take_while(|next| {
                        let next = surface
                            .cell(Point::new(*next, row))
                            .expect("the publication row stays inside its Surface");
                        matches!(next.content(), CellContent::Blank) && next.style() == cell.style()
                    })
                    .count();
                let count = u16::try_from(count)
                    .ok()
                    .and_then(NonZeroU16::new)
                    .expect("a publication blank run is nonempty and bounded by Surface width");
                output.push(TerminalOp::WriteBlank { count });
                column += count.get();
            },
            CellContent::Grapheme { text, width } => {
                output.push(TerminalOp::WriteGrapheme {
                    text,
                    width: *width,
                });
                column += width.get();
            },
            CellContent::Continuation { .. } => {
                unreachable!("publication row iteration advances over complete grapheme footprints")
            },
        }
    }
    if end < width {
        let trailing = surface
            .cell(Point::new(end, row))
            .expect("the trailing publication cell stays inside its Surface");
        if style != Some(trailing.style()) {
            output.push(TerminalOp::SetStyle(trailing.style()));
        }
    }
    output.extend([
        TerminalOp::EraseToLineEnd,
        TerminalOp::SetStyle(Style::default()),
        TerminalOp::CarriageReturn,
        TerminalOp::LineFeed,
    ]);
    output
}

fn clear_completed_rows(rows: u16) -> Vec<TerminalOp<'static>> {
    let mut output = Vec::new();
    move_up(&mut output, rows);
    move_to_column(&mut output, 0);
    let mut cursor = RelativeCursor { row: 0 };
    for row in 0..rows {
        cursor.move_to(&mut output, Point::new(0, row));
        output.push(TerminalOp::EraseLine);
    }
    cursor.move_to(&mut output, Point::new(0, 0));
    output
}

fn push<'frame>(
    operations: &mut Vec<EncodedOperation<'frame>>,
    ledger: &mut Ledger,
    terminal_ops: Vec<TerminalOp<'frame>>,
    effect: PhysicalEffect,
) {
    if terminal_ops.is_empty() {
        return;
    }
    let evidence = ledger.observe(&terminal_ops);
    let bytes = encode(&terminal_ops);
    operations.push(EncodedOperation {
        terminal_ops,
        bytes,
        effect,
        evidence,
    });
}

fn encode(operations: &[TerminalOp<'_>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    AnsiEncoder::new(&mut bytes)
        .encode_operations(operations)
        .expect("a publication plan contains no frame-size marker and encodes into memory");
    bytes
}

fn write_typed(writer: &mut impl io::Write, operations: &[TerminalOp<'_>]) -> io::Result<()> {
    writer.write_all(&encode(operations))
}

const fn previous_owned_rows(plan: InlineFramePlan) -> u16 {
    match plan {
        InlineFramePlan::Initialize { .. } => 0,
        InlineFramePlan::Update { current, .. } => current.height,
        InlineFramePlan::Reconcile { previous, .. } => previous.height,
        InlineFramePlan::Reanchor { abandoned_rows, .. } => abandoned_rows,
    }
}

const fn current_size(plan: InlineFramePlan) -> Size {
    match plan {
        InlineFramePlan::Initialize { current, .. }
        | InlineFramePlan::Update { current, .. }
        | InlineFramePlan::Reconcile { current, .. }
        | InlineFramePlan::Reanchor { current, .. } => current,
    }
}

const fn target_cursor(plan: InlineFramePlan) -> Point {
    match plan {
        InlineFramePlan::Initialize { cursor, .. }
        | InlineFramePlan::Update { cursor, .. }
        | InlineFramePlan::Reconcile { cursor, .. }
        | InlineFramePlan::Reanchor { cursor, .. } => cursor,
    }
}

fn allocate_rows<'frame>(output: &mut Vec<TerminalOp<'frame>>, rows: u16) {
    if rows == 0 {
        return;
    }
    output.push(TerminalOp::CarriageReturn);
    output.extend((0..rows).map(|_| TerminalOp::LineFeed));
}

fn move_up<'frame>(output: &mut Vec<TerminalOp<'frame>>, rows: u16) {
    if let Some(rows) = NonZeroU16::new(rows) {
        output.push(TerminalOp::MoveUp { rows });
    }
}

fn move_down<'frame>(output: &mut Vec<TerminalOp<'frame>>, rows: u16) {
    if let Some(rows) = NonZeroU16::new(rows) {
        output.push(TerminalOp::MoveDown { rows });
    }
}

fn move_to_column<'frame>(output: &mut Vec<TerminalOp<'frame>>, column: u16) {
    output.push(TerminalOp::MoveToColumn(column));
}
