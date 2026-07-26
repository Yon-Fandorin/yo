use std::{collections::TryReserveError, error::Error, fmt};

use super::{Cell, GeometryError, Point, Rect, Size, Style, SurfaceView};

/// Completed two-dimensional physical cell state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Surface {
    size: Size,
    cells: Vec<Cell>,
}

impl Surface {
    pub fn new(size: Size) -> Result<Self, SurfaceError> {
        let len = usize::from(size.width) * usize::from(size.height);
        let mut cells = Vec::new();
        cells.try_reserve_exact(len)?;
        cells.resize_with(len, || Cell::blank(Style::default()));
        Ok(Self { size, cells })
    }

    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    #[must_use]
    pub fn cell(&self, point: Point) -> Option<&Cell> {
        self.index(point).map(|index| &self.cells[index])
    }

    pub fn view(&mut self, rect: Rect) -> Result<SurfaceView<'_>, GeometryError> {
        if !rect.fits_within(self.size)? {
            return Err(GeometryError::OutOfBounds);
        }
        Ok(SurfaceView::new(self, rect))
    }

    pub(crate) fn index(&self, point: Point) -> Option<usize> {
        if point.x >= self.size.width || point.y >= self.size.height {
            return None;
        }
        Some(usize::from(point.y) * usize::from(self.size.width) + usize::from(point.x))
    }

    pub(crate) fn cell_by_index(&self, index: usize) -> &Cell {
        &self.cells[index]
    }

    pub(crate) fn replace_by_index(&mut self, index: usize, cell: Cell) {
        self.cells[index] = cell;
    }
}

/// A surface could not allocate its bounded cell storage.
#[derive(Debug)]
pub struct SurfaceError(TryReserveError);

impl fmt::Display for SurfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("surface cell storage allocation failed")
    }
}

impl Error for SurfaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<TryReserveError> for SurfaceError {
    fn from(error: TryReserveError) -> Self {
        Self(error)
    }
}
