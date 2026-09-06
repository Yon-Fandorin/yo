//! Small, terminal-safe meter primitives shared by text and TUI surfaces.
//!
//! The meter layer deliberately separates three decisions that are often mixed
//! together in command output:
//!
//! - MeterShape decides how a value occupies cells;
//! - MeterGlyphs decides which glyph family is used; and
//! - MeterTemplate decides how the rendered value is placed beside a label.
//!
//! A caller can therefore keep the same percentage semantics while changing a
//! compact one-cell level meter into a horizontal bar or a multi-line column.

const MAX_METER_CELLS: usize = 4_096;
const MAX_METER_BYTES: usize = 64 * 1024;
const MAX_METER_LEVELS: usize = 64;

mod error;
mod glyphs;
mod render;
mod shape;
mod template;

#[cfg(test)]
mod tests;

pub use error::{MeterError, MeterGlyphSlot, MeterTemplateError};
pub use glyphs::MeterGlyphs;
pub use render::{MeterSpec, format_percent};
pub use shape::MeterShape;
pub use template::MeterTemplate;
