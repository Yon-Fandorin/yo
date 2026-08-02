//! A deterministic, adapter-independent two-dimensional cell surface.

mod cell;
mod diff;
mod geometry;
mod storage;
mod style;
mod text;
mod view;

pub use cell::{Cell, CellContent};
pub use diff::{FrameDiff, RowSpan};
pub use geometry::{GeometryError, Point, Rect, Size};
pub use storage::{Surface, SurfaceError};
pub use style::{Attributes, Color, Style};
pub use text::{Grapheme, GraphemeError, WIDTH_PROFILE, cell_width};
pub use view::{SurfaceView, WriteOutcome};

#[cfg(test)]
mod tests;
