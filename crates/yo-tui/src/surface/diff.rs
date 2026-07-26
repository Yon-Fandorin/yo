use super::{Cell, CellContent, Size, Surface};

/// Deterministic changed spans between two completed surfaces.
#[derive(Debug, Eq, PartialEq)]
pub struct FrameDiff<'current> {
    previous_size: Size,
    current_size: Size,
    spans: Vec<RowSpan<'current>>,
}

impl<'current> FrameDiff<'current> {
    /// Compares complete frames without dirty-region hints.
    #[must_use]
    pub fn between(previous: &Surface, current: &'current Surface) -> Self {
        let previous_size = previous.size();
        let current_size = current.size();
        let spans = if previous_size == current_size {
            changed_spans(previous, current)
        } else {
            complete_current_rows(current)
        };

        Self {
            previous_size,
            current_size,
            spans,
        }
    }

    #[must_use]
    pub const fn previous_size(&self) -> Size {
        self.previous_size
    }

    #[must_use]
    pub const fn current_size(&self) -> Size {
        self.current_size
    }

    #[must_use]
    pub fn spans(&self) -> &[RowSpan<'current>] {
        &self.spans
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.previous_size == self.current_size && self.spans.is_empty()
    }
}

/// One changed row range containing complete current cell state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowSpan<'current> {
    row: u16,
    start_column: u16,
    cells: &'current [Cell],
}

impl<'current> RowSpan<'current> {
    #[must_use]
    pub const fn row(&self) -> u16 {
        self.row
    }

    #[must_use]
    pub const fn start_column(&self) -> u16 {
        self.start_column
    }

    #[must_use]
    pub fn end_column(&self) -> u16 {
        self.start_column
            + u16::try_from(self.cells.len()).expect("a row span cannot exceed surface width")
    }

    #[must_use]
    pub const fn cells(&self) -> &'current [Cell] {
        self.cells
    }
}

fn changed_spans<'current>(
    previous: &Surface,
    current: &'current Surface,
) -> Vec<RowSpan<'current>> {
    let size = current.size();
    let mut spans = Vec::new();

    for row in 0..size.height {
        let mut column = 0;
        while column < size.width {
            let Some(changed_start) =
                next_changed_column(previous, current, row, column, size.width)
            else {
                break;
            };
            let changed_end = next_equal_column(previous, current, row, changed_start, size.width);
            let (start, end) =
                expand_to_grapheme_boundaries(previous, current, row, changed_start, changed_end);
            spans.push(row_span(current, row, start, end));
            column = end;
        }
    }

    spans
}

fn complete_current_rows(current: &Surface) -> Vec<RowSpan<'_>> {
    let size = current.size();
    if size.width == 0 {
        return Vec::new();
    }

    (0..size.height)
        .map(|row| row_span(current, row, 0, size.width))
        .collect()
}

fn next_changed_column(
    previous: &Surface,
    current: &Surface,
    row: u16,
    start: u16,
    width: u16,
) -> Option<u16> {
    (start..width).find(|&column| cells_differ(previous, current, row, column))
}

fn next_equal_column(
    previous: &Surface,
    current: &Surface,
    row: u16,
    start: u16,
    width: u16,
) -> u16 {
    (start..width)
        .find(|&column| !cells_differ(previous, current, row, column))
        .unwrap_or(width)
}

fn cells_differ(previous: &Surface, current: &Surface, row: u16, column: u16) -> bool {
    let index = row_index(current.size(), row, column);
    previous.cell_by_index(index) != current.cell_by_index(index)
}

fn expand_to_grapheme_boundaries(
    previous: &Surface,
    current: &Surface,
    row: u16,
    start: u16,
    end: u16,
) -> (u16, u16) {
    let mut expanded_start = start;
    let mut expanded_end = end;

    for column in start..end {
        for surface in [previous, current] {
            let (footprint_start, footprint_end) = footprint(surface, row, column);
            expanded_start = expanded_start.min(footprint_start);
            expanded_end = expanded_end.max(footprint_end);
        }
    }

    (expanded_start, expanded_end)
}

fn footprint(surface: &Surface, row: u16, column: u16) -> (u16, u16) {
    let cell = surface.cell_by_index(row_index(surface.size(), row, column));
    match cell.content() {
        CellContent::Blank => (column, column + 1),
        CellContent::Grapheme { width, .. } => (column, column + width.get()),
        CellContent::Continuation { back } => {
            let leader = column
                .checked_sub(back.get())
                .expect("Surface invariant: continuation has a preceding leader");
            let leader_cell = surface.cell_by_index(row_index(surface.size(), row, leader));
            let CellContent::Grapheme { width, .. } = leader_cell.content() else {
                panic!("Surface invariant: continuation points to a grapheme leader");
            };
            (leader, leader + width.get())
        },
    }
}

fn row_span(current: &Surface, row: u16, start: u16, end: u16) -> RowSpan<'_> {
    let size = current.size();
    let start_index = row_index(size, row, start);
    let end_index = row_index(size, row, end);
    RowSpan {
        row,
        start_column: start,
        cells: current.cells_by_range(start_index..end_index),
    }
}

fn row_index(size: Size, row: u16, column: u16) -> usize {
    usize::from(row) * usize::from(size.width) + usize::from(column)
}
