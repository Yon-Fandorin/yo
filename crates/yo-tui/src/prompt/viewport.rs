//! Stateful selection of the prompt rows visible inside a bounded view.

use std::num::NonZeroU16;

use crate::surface::Point;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PromptViewState {
    first_visible_row: u16,
}

impl PromptViewState {
    pub(super) fn set_first_visible_row(&mut self, row: u16) {
        self.first_visible_row = row;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct VisibleRows {
    first: u16,
    end: u16,
}

impl VisibleRows {
    pub(super) fn for_cursor(
        content_height: NonZeroU16,
        cursor_row: u16,
        available_height: NonZeroU16,
        state: PromptViewState,
    ) -> Self {
        let content_height = content_height.get();
        let available_height = available_height.get();

        if content_height <= available_height {
            return Self {
                first: 0,
                end: content_height,
            };
        }

        let maximum_first = content_height - available_height;
        let mut first = state.first_visible_row.min(maximum_first);
        let mut end = first + available_height;

        if cursor_row < first {
            first = cursor_row;
            end = first + available_height;
        } else if cursor_row >= end {
            first = cursor_row + 1 - available_height;
            end = first + available_height;
        }

        Self { first, end }
    }

    pub(super) const fn first(self) -> u16 {
        self.first
    }

    pub(super) const fn contains(self, row: u16) -> bool {
        row >= self.first && row < self.end
    }

    pub(super) const fn translate(self, point: Point) -> Point {
        Point::new(point.x, point.y - self.first)
    }
}
