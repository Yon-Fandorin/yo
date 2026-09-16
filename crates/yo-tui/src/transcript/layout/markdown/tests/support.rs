pub(super) use std::{io::Cursor, iter};

pub(super) use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) use super::super::{image::decode_image, *};
pub(super) use crate::surface::Size;

pub(super) fn rows(prepared: &PreparedMarkdown) -> Vec<String> {
    let mut rows = vec![String::new(); usize::from(prepared.height)];
    for glyph in &prepared.glyphs {
        rows[usize::from(glyph.point.y)].push_str(glyph.grapheme.as_str());
    }
    rows
}
