//! Grapheme-aware editable text without key-binding policy.

use unicode_segmentation::UnicodeSegmentation;

/// Text and a cursor kept on an extended grapheme-cluster boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TextBuffer {
    text: String,
    cursor: usize,
}

impl TextBuffer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) const fn cursor_byte_index(&self) -> usize {
        self.cursor
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub(crate) fn clear(&mut self) -> bool {
        if self.text.is_empty() {
            return false;
        }

        self.text.clear();
        self.cursor = 0;
        true
    }

    pub(crate) fn take(&mut self) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }

        self.cursor = 0;
        Some(std::mem::take(&mut self.text))
    }

    pub(crate) fn insert(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }

        let insertion_end = self.cursor + text.len();
        self.text.insert_str(self.cursor, text);
        self.cursor = boundary_at_or_after(&self.text, insertion_end);
        true
    }

    pub(crate) fn move_left(&mut self) -> bool {
        let Some(previous) = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
        else {
            return false;
        };

        self.cursor = previous;
        true
    }

    pub(crate) fn move_right(&mut self) -> bool {
        let Some(next) = next_boundary(&self.text, self.cursor) else {
            return false;
        };

        self.cursor = next;
        true
    }

    pub(crate) fn delete_backward(&mut self) -> bool {
        let Some(previous) = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
        else {
            return false;
        };

        self.text.drain(previous..self.cursor);
        self.cursor = boundary_at_or_after(&self.text, previous);
        true
    }

    pub(crate) fn delete_forward(&mut self) -> bool {
        let Some(next) = next_boundary(&self.text, self.cursor) else {
            return false;
        };

        self.text.drain(self.cursor..next);
        self.cursor = boundary_at_or_after(&self.text, self.cursor);
        true
    }
}

fn boundary_at_or_after(text: &str, byte_index: usize) -> usize {
    if byte_index == 0 {
        return 0;
    }

    text.grapheme_indices(true)
        .map(|(index, grapheme)| index + grapheme.len())
        .find(|&boundary| boundary >= byte_index)
        .unwrap_or(text.len())
}

fn next_boundary(text: &str, cursor: usize) -> Option<usize> {
    text[cursor..]
        .graphemes(true)
        .next()
        .map(|grapheme| cursor + grapheme.len())
}

#[cfg(test)]
mod tests;
