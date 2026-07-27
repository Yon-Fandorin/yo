use std::{collections::BTreeSet, num::NonZeroU16};

use super::{Cell, CellContent, GeometryError, Grapheme, Point, Rect, Style, Surface};

/// Result of a bounded, atomic surface mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteOutcome {
    Written,
    Clipped,
}

/// Mutable operations restricted to one assigned rectangle.
pub struct SurfaceView<'surface> {
    surface: &'surface mut Surface,
    rect: Rect,
}

impl<'surface> SurfaceView<'surface> {
    pub(crate) const fn new(surface: &'surface mut Surface, rect: Rect) -> Self {
        Self { surface, rect }
    }

    #[must_use]
    pub const fn size(&self) -> super::Size {
        self.rect.size
    }

    #[must_use]
    pub fn cell(&self, point: Point) -> Option<&Cell> {
        self.absolute(point)
            .and_then(|absolute| self.surface.cell(absolute))
    }

    pub fn clear(&mut self, style: Style) -> WriteOutcome {
        let indexes = self.view_indexes();
        let Some(region) = self.closed_mutation_region(indexes) else {
            return WriteOutcome::Clipped;
        };
        for index in region {
            self.surface.replace_by_index(index, Cell::blank(style));
        }
        WriteOutcome::Written
    }

    pub(crate) fn subview(&mut self, rect: Rect) -> Result<SurfaceView<'_>, GeometryError> {
        if !rect.fits_within(self.rect.size)? {
            return Err(GeometryError::OutOfBounds);
        }
        let origin = Point::new(
            self.rect
                .origin
                .x
                .checked_add(rect.origin.x)
                .ok_or(GeometryError::Overflow)?,
            self.rect
                .origin
                .y
                .checked_add(rect.origin.y)
                .ok_or(GeometryError::Overflow)?,
        );
        Ok(SurfaceView::new(self.surface, Rect::new(origin, rect.size)))
    }

    pub fn write(&mut self, point: Point, grapheme: Grapheme, style: Style) -> WriteOutcome {
        let width = grapheme.width().get();
        let Some(end_x) = point.x.checked_add(width) else {
            return WriteOutcome::Clipped;
        };
        if point.y >= self.rect.size.height || end_x > self.rect.size.width {
            return WriteOutcome::Clipped;
        }

        let proposed = (0..width)
            .filter_map(|offset| self.absolute(Point::new(point.x + offset, point.y)))
            .filter_map(|absolute| self.surface.index(absolute))
            .collect::<BTreeSet<_>>();
        let Some(region) = self.closed_mutation_region(proposed.iter().copied()) else {
            return WriteOutcome::Clipped;
        };

        for index in region {
            self.surface.replace_by_index(index, Cell::blank(style));
        }

        let (text, width) = grapheme.into_parts();
        let leader = *proposed
            .first()
            .expect("a validated grapheme has a nonzero footprint");
        self.surface
            .replace_by_index(leader, Cell::grapheme(text, width, style));
        for (back, index) in proposed.iter().copied().skip(1).enumerate() {
            let back = u16::try_from(back + 1)
                .ok()
                .and_then(NonZeroU16::new)
                .expect("grapheme width is bounded by u16");
            self.surface
                .replace_by_index(index, Cell::continuation(back, style));
        }
        WriteOutcome::Written
    }

    fn absolute(&self, point: Point) -> Option<Point> {
        if point.x >= self.rect.size.width || point.y >= self.rect.size.height {
            return None;
        }
        Some(Point::new(
            self.rect.origin.x.checked_add(point.x)?,
            self.rect.origin.y.checked_add(point.y)?,
        ))
    }

    fn view_indexes(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.rect.size.height).flat_map(move |y| {
            (0..self.rect.size.width).filter_map(move |x| {
                self.absolute(Point::new(x, y))
                    .and_then(|point| self.surface.index(point))
            })
        })
    }

    fn closed_mutation_region(
        &self,
        seeds: impl IntoIterator<Item = usize>,
    ) -> Option<BTreeSet<usize>> {
        let mut region = BTreeSet::new();
        for index in seeds {
            region.extend(self.footprint(index)?);
        }
        Some(region)
    }

    fn footprint(&self, index: usize) -> Option<Vec<usize>> {
        let surface_width = usize::from(self.surface.size().width);
        let row = index / surface_width;
        let column = index % surface_width;
        let (leader_column, width) = match self.surface.cell_by_index(index).content() {
            CellContent::Blank => return Some(vec![index]),
            CellContent::Grapheme { width, .. } => (column, usize::from(width.get())),
            CellContent::Continuation { back } => {
                let leader_column = column.checked_sub(usize::from(back.get()))?;
                let leader_index = row.checked_mul(surface_width)?.checked_add(leader_column)?;
                match self.surface.cell_by_index(leader_index).content() {
                    CellContent::Grapheme { width, .. } => {
                        (leader_column, usize::from(width.get()))
                    },
                    _ => return None,
                }
            },
        };

        let indexes = (leader_column..leader_column.checked_add(width)?)
            .map(|column| row * surface_width + column)
            .collect::<Vec<_>>();
        indexes
            .iter()
            .all(|index| self.contains_index(*index))
            .then_some(indexes)
    }

    fn contains_index(&self, index: usize) -> bool {
        let surface_width = usize::from(self.surface.size().width);
        let point = Point::new(
            u16::try_from(index % surface_width).expect("surface coordinates fit u16"),
            u16::try_from(index / surface_width).expect("surface coordinates fit u16"),
        );
        let end_x = self.rect.end_x().expect("validated view geometry");
        let end_y = self.rect.end_y().expect("validated view geometry");
        point.x >= self.rect.origin.x
            && point.x < end_x
            && point.y >= self.rect.origin.y
            && point.y < end_y
    }
}
