use std::num::NonZeroU16;

use crate::surface::{Cell, CellContent, FrameDiff, Point, Size, Style};

/// One terminal effect whose meaning is independent of its byte encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalOp<'frame> {
    /// Delegates owned-region reconciliation to the outer mode controller.
    FrameSizeChanged {
        previous: Size,
        current: Size,
    },
    MoveTo(Point),
    SetStyle(Style),
    WriteGrapheme {
        text: &'frame str,
        width: NonZeroU16,
    },
    WriteBlank {
        count: NonZeroU16,
    },
}

/// Deterministic terminal operations compiled from a completed-frame diff.
#[derive(Debug, Eq, PartialEq)]
pub struct TerminalOps<'frame> {
    operations: Vec<TerminalOp<'frame>>,
}

impl<'frame> TerminalOps<'frame> {
    #[must_use]
    pub fn from_diff(diff: &FrameDiff<'frame>) -> Self {
        let mut compiler = Compiler::default();
        if diff.previous_size() != diff.current_size() {
            compiler.operations.push(TerminalOp::FrameSizeChanged {
                previous: diff.previous_size(),
                current: diff.current_size(),
            });
        }
        for span in diff.spans() {
            compiler.operations.push(TerminalOp::MoveTo(Point::new(
                span.start_column(),
                span.row(),
            )));
            compiler.compile_cells(span.cells());
        }
        Self {
            operations: compiler.operations,
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[TerminalOp<'frame>] {
        &self.operations
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

#[derive(Default)]
struct Compiler<'frame> {
    operations: Vec<TerminalOp<'frame>>,
    selected_style: Option<Style>,
}

impl<'frame> Compiler<'frame> {
    fn compile_cells(&mut self, cells: &'frame [Cell]) {
        let mut offset = 0;
        while offset < cells.len() {
            let cell = &cells[offset];
            match cell.content() {
                CellContent::Blank => {
                    self.select_style(cell.style());
                    let count = blank_run_length(&cells[offset..], cell.style());
                    self.operations.push(TerminalOp::WriteBlank {
                        count: nonzero_count(count),
                    });
                    offset += count;
                },
                CellContent::Grapheme { text, width } => {
                    self.select_style(cell.style());
                    assert_complete_footprint(&cells[offset..], *width, cell.style());
                    self.operations.push(TerminalOp::WriteGrapheme {
                        text,
                        width: *width,
                    });
                    offset += usize::from(width.get());
                },
                CellContent::Continuation { .. } => {
                    panic!("FrameDiff invariant: a row span cannot begin inside a grapheme");
                },
            }
        }
    }

    fn select_style(&mut self, style: Style) {
        if self.selected_style == Some(style) {
            return;
        }
        self.operations.push(TerminalOp::SetStyle(style));
        self.selected_style = Some(style);
    }
}

fn blank_run_length(cells: &[Cell], style: Style) -> usize {
    cells
        .iter()
        .take_while(|cell| cell.content() == &CellContent::Blank && cell.style() == style)
        .count()
}

fn assert_complete_footprint(cells: &[Cell], width: NonZeroU16, style: Style) {
    let width = usize::from(width.get());
    assert!(
        cells.len() >= width,
        "FrameDiff invariant: grapheme footprint crosses its row span"
    );
    for (index, cell) in cells.iter().take(width).enumerate().skip(1) {
        assert!(
            matches!(
                cell.content(),
                CellContent::Continuation { back }
                    if usize::from(back.get()) == index && cell.style() == style
            ),
            "Surface invariant: grapheme continuation does not match its leader"
        );
    }
}

fn nonzero_count(count: usize) -> NonZeroU16 {
    u16::try_from(count)
        .ok()
        .and_then(NonZeroU16::new)
        .expect("a non-empty row run cannot exceed surface width")
}
