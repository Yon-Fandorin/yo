//! Cell occupancy shapes for percentage meters.

/// The visual shape used to represent one bounded percentage value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeterShape {
    /// One graduated cell, useful in dense tables and status lines.
    VerticalLevel,
    /// A fixed-width left-to-right bar.
    HorizontalBar {
        /// Number of terminal cells in the bar.
        width: usize,
    },
    /// A fixed-height bottom-up bar rendered from top to bottom, one row per line.
    VerticalBar {
        /// Number of terminal rows in the bar.
        height: usize,
    },
}
