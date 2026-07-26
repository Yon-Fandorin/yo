use std::{error::Error, fmt};

/// A zero-based position inside a surface.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

impl Point {
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

/// The physical dimensions of a surface or view.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    #[must_use]
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

/// A rectangular region with a zero-based origin.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    pub(crate) fn end_x(self) -> Result<u16, GeometryError> {
        self.origin
            .x
            .checked_add(self.size.width)
            .ok_or(GeometryError::Overflow)
    }

    pub(crate) fn end_y(self) -> Result<u16, GeometryError> {
        self.origin
            .y
            .checked_add(self.size.height)
            .ok_or(GeometryError::Overflow)
    }

    pub(crate) fn fits_within(self, size: Size) -> Result<bool, GeometryError> {
        Ok(self.end_x()? <= size.width && self.end_y()? <= size.height)
    }
}

/// A geometry calculation could not produce a valid bounded region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    Overflow,
    OutOfBounds,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow => formatter.write_str("geometry arithmetic overflowed"),
            Self::OutOfBounds => formatter.write_str("rectangle is outside the surface"),
        }
    }
}

impl Error for GeometryError {}
