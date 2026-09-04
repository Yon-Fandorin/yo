//! Built-in and custom terminal-safe meter glyph families.

use super::{
    MAX_METER_BYTES, MAX_METER_LEVELS,
    error::{MeterError, MeterGlyphSlot},
    shape::MeterShape,
};
use crate::{GlyphProfile, surface::Grapheme};

// Rich vertical levels stop at seven-eighths so adjacent terminal rows retain a
// small visual inset instead of merging into one solid block.
const RICH_LEVELS: &[&str] = &["▁", "▂", "▃", "▄", "▅", "▆", "▇"];
const ASCII_LEVELS: &[&str] = &[".", ":", "-", "=", "+", "*", "#", "@"];

/// Glyphs used by MeterShape renderers.
///
/// filled and empty must each be one terminal cell. Every levels entry must
/// also be one terminal cell. The level set is used by
/// MeterShape::VerticalLevel and must contain at least one entry for that
/// shape. VerticalBar uses vertical_filled explicitly; [`Self::new`] defaults
/// it to filled so custom level palettes do not silently change bar output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeterGlyphs<'a> {
    /// Glyph repeated for occupied cells in horizontal bar shapes.
    pub filled: &'a str,
    /// Glyph repeated for unoccupied cells in bar shapes.
    pub empty: &'a str,
    /// Glyph repeated for occupied cells in vertical bar shapes.
    pub vertical_filled: &'a str,
    /// Graduated one-cell glyphs used by [`MeterShape::VerticalLevel`].
    pub levels: &'a [&'a str],
}

impl MeterGlyphs<'static> {
    /// Returns the built-in glyph family matching the shared appearance profile.
    #[must_use]
    pub const fn for_profile(profile: GlyphProfile) -> Self {
        match profile {
            GlyphProfile::Rich => Self {
                filled: "█",
                empty: "░",
                vertical_filled: "▇",
                levels: RICH_LEVELS,
            },
            GlyphProfile::Ascii => Self {
                filled: "#",
                empty: "-",
                vertical_filled: "#",
                levels: ASCII_LEVELS,
            },
        }
    }
}

impl<'a> MeterGlyphs<'a> {
    /// Creates a glyph family for a custom theme.
    #[must_use]
    pub const fn new(filled: &'a str, empty: &'a str, levels: &'a [&'a str]) -> Self {
        Self {
            filled,
            empty,
            vertical_filled: filled,
            levels,
        }
    }

    /// Replaces the explicit glyph used by [`MeterShape::VerticalBar`].
    #[must_use]
    pub const fn with_vertical_filled(self, vertical_filled: &'a str) -> Self {
        Self {
            vertical_filled,
            ..self
        }
    }
}

pub(super) fn validate_glyphs(
    glyphs: MeterGlyphs<'_>,
    shape: MeterShape,
) -> Result<(), MeterError> {
    if matches!(shape, MeterShape::VerticalLevel) {
        if glyphs.levels.is_empty() {
            return Err(MeterError::EmptyLevels);
        }
        if glyphs.levels.len() > MAX_METER_LEVELS {
            return Err(MeterError::TooManyLevels {
                count: glyphs.levels.len(),
            });
        }
    }
    validate_glyph(glyphs.filled, MeterGlyphSlot::Filled, None)?;
    validate_glyph(glyphs.empty, MeterGlyphSlot::Empty, None)?;
    if matches!(shape, MeterShape::VerticalLevel) {
        for (index, glyph) in glyphs.levels.iter().enumerate() {
            validate_glyph(glyph, MeterGlyphSlot::Level, Some(index))?;
        }
    } else if matches!(shape, MeterShape::VerticalBar { .. }) {
        validate_glyph(glyphs.vertical_filled, MeterGlyphSlot::VerticalFilled, None)?;
    }
    Ok(())
}

fn validate_glyph(
    glyph: &str,
    slot: MeterGlyphSlot,
    index: Option<usize>,
) -> Result<(), MeterError> {
    if glyph.len() > MAX_METER_BYTES {
        return Err(MeterError::RenderTooLarge {
            cells: 1,
            bytes: glyph.len(),
        });
    }
    let grapheme = Grapheme::try_from(glyph).map_err(|cause| MeterError::InvalidGlyph {
        slot,
        index,
        cause,
    })?;
    let width = grapheme.width().get();
    if width != 1 {
        return Err(MeterError::GlyphMustBeOneCell { slot, index, width });
    }
    Ok(())
}
