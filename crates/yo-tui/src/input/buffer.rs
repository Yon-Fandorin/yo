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

    pub(crate) fn replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &str,
    ) -> bool {
        if range.start > range.end
            || range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return false;
        }
        if self.text[range.clone()] == *replacement {
            self.cursor = boundary_at_or_after(&self.text, range.start + replacement.len());
            return false;
        }
        self.text.replace_range(range.clone(), replacement);
        self.cursor = boundary_at_or_after(&self.text, range.start + replacement.len());
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

    pub(crate) fn move_word_start(&mut self) -> bool {
        let start = self.word_start();
        let changed = self.cursor != start;
        self.cursor = start;
        changed
    }

    pub(crate) fn move_word_end(&mut self) -> bool {
        let mut end = self.cursor;
        let mut found_word = false;
        for (offset, grapheme) in self.text[self.cursor..].grapheme_indices(true) {
            let whitespace = grapheme.chars().all(char::is_whitespace);
            if found_word && whitespace {
                break;
            }
            found_word |= !whitespace;
            end = self.cursor + offset + grapheme.len();
        }
        let changed = self.cursor != end;
        self.cursor = end;
        changed
    }

    pub(crate) fn kill_word_start(&mut self) -> Option<String> {
        let start = self.word_start();
        (start != self.cursor).then(|| self.kill_range(start..self.cursor))
    }

    fn word_start(&self) -> usize {
        let mut start = self.cursor;
        let mut found_word = false;
        for (offset, grapheme) in self.text[..self.cursor].grapheme_indices(true).rev() {
            let whitespace = grapheme.chars().all(char::is_whitespace);
            if found_word && whitespace {
                break;
            }
            found_word |= !whitespace;
            start = offset;
        }
        start
    }

    pub(crate) fn move_line_start(&mut self) -> bool {
        let start = self.line_start();
        let changed = self.cursor != start;
        self.cursor = start;
        changed
    }

    pub(crate) fn move_line_end(&mut self) -> bool {
        let end = self.line_end();
        let changed = self.cursor != end;
        self.cursor = end;
        changed
    }

    pub(crate) fn kill_line_start(&mut self) -> Option<String> {
        let start = self.line_start();
        let start = if start == self.cursor {
            self.text[..self.cursor]
                .grapheme_indices(true)
                .next_back()?
                .0
        } else {
            start
        };
        Some(self.kill_range(start..self.cursor))
    }

    pub(crate) fn kill_line_end(&mut self) -> Option<String> {
        let end = self.line_end();
        let end = if end == self.cursor {
            next_boundary(&self.text, self.cursor)?
        } else {
            end
        };
        Some(self.kill_range(self.cursor..end))
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .grapheme_indices(true)
            .find(|(_, grapheme)| grapheme.contains('\n'))
            .map_or(self.text.len(), |(index, _)| self.cursor + index)
    }

    fn kill_range(&mut self, range: std::ops::Range<usize>) -> String {
        let start = range.start;
        let removed = self.text.drain(range).collect();
        self.cursor = boundary_at_or_after(&self.text, start);
        removed
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
