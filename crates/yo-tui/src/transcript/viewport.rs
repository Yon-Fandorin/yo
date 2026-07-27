//! Semantic transcript scrolling independent of terminal key bindings.

use std::num::NonZeroU16;

use crate::surface::Point;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum TranscriptViewMode {
    #[default]
    FollowTail,
    Detached,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TranscriptViewState {
    mode: TranscriptViewMode,
    first_visible_row: u16,
}

impl TranscriptViewState {
    pub(crate) const fn mode(self) -> TranscriptViewMode {
        self.mode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptScrollCommand {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    JumpToStart,
    JumpToTail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct VisibleRows {
    first: u16,
    end: u16,
    mode: TranscriptViewMode,
}

impl VisibleRows {
    pub(super) fn resolve(
        content_height: u16,
        available_height: NonZeroU16,
        state: TranscriptViewState,
        command: Option<TranscriptScrollCommand>,
    ) -> Self {
        let available_height = available_height.get();
        let maximum_first = content_height.saturating_sub(available_height);
        let current = match state.mode {
            TranscriptViewMode::FollowTail => maximum_first,
            TranscriptViewMode::Detached => state.first_visible_row.min(maximum_first),
        };
        let page = available_height.saturating_sub(1).max(1);
        let (first, mode) = match command {
            None => (current, state.mode),
            Some(TranscriptScrollCommand::LineUp) => away_from_tail(current, 1, state.mode),
            Some(TranscriptScrollCommand::LineDown) => toward_tail(current, 1, maximum_first),
            Some(TranscriptScrollCommand::PageUp) => away_from_tail(current, page, state.mode),
            Some(TranscriptScrollCommand::PageDown) => toward_tail(current, page, maximum_first),
            Some(TranscriptScrollCommand::JumpToStart) => (0, TranscriptViewMode::Detached),
            Some(TranscriptScrollCommand::JumpToTail) => {
                (maximum_first, TranscriptViewMode::FollowTail)
            },
        };

        Self {
            first,
            end: first.saturating_add(available_height),
            mode,
        }
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

    pub(super) const fn next_state(self) -> TranscriptViewState {
        TranscriptViewState {
            mode: self.mode,
            first_visible_row: self.first,
        }
    }
}

fn away_from_tail(
    current: u16,
    amount: u16,
    current_mode: TranscriptViewMode,
) -> (u16, TranscriptViewMode) {
    let next = current.saturating_sub(amount);
    if next == current {
        (next, current_mode)
    } else {
        (next, TranscriptViewMode::Detached)
    }
}

fn toward_tail(current: u16, amount: u16, maximum_first: u16) -> (u16, TranscriptViewMode) {
    let next = current.saturating_add(amount).min(maximum_first);
    if next == maximum_first {
        (next, TranscriptViewMode::FollowTail)
    } else {
        (next, TranscriptViewMode::Detached)
    }
}
