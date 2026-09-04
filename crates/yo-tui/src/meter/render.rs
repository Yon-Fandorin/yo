//! Percentage normalization and shape-specific meter rendering.

use super::{
    MAX_METER_BYTES, MAX_METER_CELLS,
    error::MeterError,
    glyphs::{MeterGlyphs, validate_glyphs},
    shape::MeterShape,
    template::MeterTemplate,
};

const BASIS_POINTS_PER_PERCENT: u16 = 100;
const FULL_PERCENT_BASIS_POINTS: u16 = 100 * BASIS_POINTS_PER_PERCENT;

/// A complete reusable meter definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeterSpec<'a> {
    /// Cell-occupancy shape used to draw the value.
    pub shape: MeterShape,
    /// Terminal-safe glyph family used by the shape.
    pub glyphs: MeterGlyphs<'a>,
    /// Layout template applied around the rendered glyphs.
    pub template: MeterTemplate<'a>,
}

impl<'a> MeterSpec<'a> {
    /// Creates a meter definition from independent shape, glyph, and layout choices.
    #[must_use]
    pub const fn new(
        shape: MeterShape,
        glyphs: MeterGlyphs<'a>,
        template: MeterTemplate<'a>,
    ) -> Self {
        Self {
            shape,
            glyphs,
            template,
        }
    }

    /// Creates a raw meter definition whose template is the meter glyph alone.
    #[must_use]
    pub const fn raw(shape: MeterShape, glyphs: MeterGlyphs<'a>) -> Self {
        Self::new(shape, glyphs, MeterTemplate::new("{meter}"))
    }

    /// Replaces only the shape while preserving the glyph family and template.
    #[must_use]
    pub const fn with_shape(self, shape: MeterShape) -> Self {
        Self { shape, ..self }
    }

    /// Replaces only the glyph family while preserving the shape and template.
    #[must_use]
    pub const fn with_glyphs<'b>(self, glyphs: MeterGlyphs<'b>) -> MeterSpec<'b>
    where
        'a: 'b,
    {
        MeterSpec {
            shape: self.shape,
            glyphs,
            template: self.template,
        }
    }

    /// Replaces only the layout template while preserving the shape and glyphs.
    #[must_use]
    pub const fn with_template<'b>(self, template: MeterTemplate<'b>) -> MeterSpec<'b>
    where
        'a: 'b,
    {
        MeterSpec {
            shape: self.shape,
            glyphs: self.glyphs,
            template,
        }
    }

    /// Renders the raw meter glyphs without applying the layout template.
    pub fn render_glyph(self, percent_basis_points: u16) -> Result<String, MeterError> {
        let percent_basis_points = percent_basis_points.min(FULL_PERCENT_BASIS_POINTS);
        match self.shape {
            MeterShape::VerticalLevel => {
                validate_glyphs(self.glyphs, self.shape)?;
                let glyph = vertical_level(percent_basis_points, self.glyphs.levels)?;
                let mut output = bounded_output(1, glyph.len())?;
                output.push_str(glyph);
                Ok(output)
            },
            MeterShape::HorizontalBar { width } => {
                if width == 0 {
                    return Err(MeterError::ZeroWidth);
                }
                let filled = (u128::from(percent_basis_points) * width as u128
                    / u128::from(FULL_PERCENT_BASIS_POINTS)) as usize;
                let empty = width - filled;
                let bytes = output_bytes(
                    width,
                    filled,
                    self.glyphs.filled,
                    empty,
                    self.glyphs.empty,
                    0,
                )?;
                validate_glyphs(self.glyphs, self.shape)?;
                let mut output = bounded_output(width, bytes)?;
                for _ in 0..filled {
                    output.push_str(self.glyphs.filled);
                }
                for _ in 0..empty {
                    output.push_str(self.glyphs.empty);
                }
                Ok(output)
            },
            MeterShape::VerticalBar { height } => {
                if height == 0 {
                    return Err(MeterError::ZeroHeight);
                }
                let filled_glyph = self.glyphs.vertical_filled;
                let filled = (u128::from(percent_basis_points) * height as u128
                    / u128::from(FULL_PERCENT_BASIS_POINTS)) as usize;
                let empty = height - filled;
                let bytes = output_bytes(
                    height,
                    filled,
                    filled_glyph,
                    empty,
                    self.glyphs.empty,
                    height - 1,
                )?;
                validate_glyphs(self.glyphs, self.shape)?;
                let mut output = bounded_output(height, bytes)?;
                for row in 0..height {
                    if row != 0 {
                        output.push('\n');
                    }
                    if row < height - filled {
                        output.push_str(self.glyphs.empty);
                    } else {
                        output.push_str(filled_glyph);
                    }
                }
                Ok(output)
            },
        }
    }

    /// Renders the raw meter and expands the configured layout template.
    pub fn render(self, label: &str, percent_basis_points: u16) -> Result<String, MeterError> {
        let meter = self.render_glyph(percent_basis_points)?;
        self.template
            .render(label, &meter, percent_basis_points)
            .map_err(MeterError::Template)
    }
}

fn output_bytes(
    cells: usize,
    filled: usize,
    filled_glyph: &str,
    empty: usize,
    empty_glyph: &str,
    separator_bytes: usize,
) -> Result<usize, MeterError> {
    let bytes = filled_glyph
        .len()
        .checked_mul(filled)
        .and_then(|filled_bytes| {
            empty_glyph
                .len()
                .checked_mul(empty)
                .and_then(|empty_bytes| filled_bytes.checked_add(empty_bytes))
        })
        .and_then(|glyph_bytes| glyph_bytes.checked_add(separator_bytes))
        .ok_or(MeterError::RenderTooLarge {
            cells,
            bytes: usize::MAX,
        })?;
    ensure_bounded(cells, bytes)?;
    Ok(bytes)
}

fn bounded_output(cells: usize, bytes: usize) -> Result<String, MeterError> {
    ensure_bounded(cells, bytes)?;
    let mut output = String::new();
    output
        .try_reserve(bytes)
        .map_err(|_| MeterError::RenderTooLarge { cells, bytes })?;
    Ok(output)
}

fn ensure_bounded(cells: usize, bytes: usize) -> Result<(), MeterError> {
    if cells > MAX_METER_CELLS || bytes > MAX_METER_BYTES {
        return Err(MeterError::RenderTooLarge { cells, bytes });
    }
    Ok(())
}

/// Formats a percentage stored in hundredths of a percent without noisy trailing zeroes.
#[must_use]
pub fn format_percent(percent_basis_points: u16) -> String {
    let percent_basis_points = percent_basis_points.min(FULL_PERCENT_BASIS_POINTS);
    let whole = percent_basis_points / BASIS_POINTS_PER_PERCENT;
    let fractional = percent_basis_points % BASIS_POINTS_PER_PERCENT;
    if fractional == 0 {
        whole.to_string()
    } else if fractional.is_multiple_of(10) {
        format!("{whole}.{}", fractional / 10)
    } else {
        format!("{whole}.{fractional:02}")
    }
}

fn vertical_level<'a>(
    percent_basis_points: u16,
    levels: &'a [&'a str],
) -> Result<&'a str, MeterError> {
    let last = levels.len() - 1;
    let index = (u128::from(percent_basis_points) * last as u128
        + u128::from(FULL_PERCENT_BASIS_POINTS / 2))
        / u128::from(FULL_PERCENT_BASIS_POINTS);
    let index = usize::try_from(index).map_err(|_| MeterError::TooManyLevels {
        count: levels.len(),
    })?;
    Ok(levels[index])
}
