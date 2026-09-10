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
    first_visible_row: usize,
}

impl TranscriptViewState {
    pub(crate) const fn mode(self) -> TranscriptViewMode {
        self.mode
    }

    #[cfg(test)]
    pub(crate) const fn first_visible_row(self) -> usize {
        self.first_visible_row
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptScrollCommand {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    PreviousItem,
    NextItem,
    JumpToStart,
    JumpToTail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct VisibleRows {
    first: usize,
    end: usize,
    mode: TranscriptViewMode,
}

impl VisibleRows {
    pub(super) fn resolve_commands(
        content_height: usize,
        available_height: NonZeroU16,
        state: TranscriptViewState,
        commands: &[TranscriptScrollCommand],
        item_starts: &[usize],
    ) -> Self {
        let mut visible = Self::resolve(content_height, available_height, state, None, item_starts);
        for command in commands {
            visible = Self::resolve(
                content_height,
                available_height,
                visible.next_state(),
                Some(*command),
                item_starts,
            );
        }
        visible
    }
    pub(super) fn resolve(
        content_height: usize,
        available_height: NonZeroU16,
        state: TranscriptViewState,
        command: Option<TranscriptScrollCommand>,
        item_starts: &[usize],
    ) -> Self {
        let available_height = usize::from(available_height.get());
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
            Some(TranscriptScrollCommand::PreviousItem) => (
                item_starts
                    .iter()
                    .rev()
                    .copied()
                    .find(|&row| row < current)
                    .unwrap_or(0),
                TranscriptViewMode::Detached,
            ),
            Some(TranscriptScrollCommand::NextItem) => {
                let next = item_starts
                    .iter()
                    .copied()
                    .find(|&row| row > current)
                    .unwrap_or(maximum_first)
                    .min(maximum_first);
                (
                    next,
                    if next == maximum_first {
                        TranscriptViewMode::FollowTail
                    } else {
                        TranscriptViewMode::Detached
                    },
                )
            },
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

    pub(super) const fn first(self) -> usize {
        self.first
    }

    pub(super) const fn end(self) -> usize {
        self.end
    }

    pub(super) const fn follows_tail(self) -> bool {
        matches!(self.mode, TranscriptViewMode::FollowTail)
    }

    pub(super) const fn contains(self, row: usize) -> bool {
        row >= self.first && row < self.end
    }

    pub(super) fn translate(self, x: u16, row: usize) -> Point {
        debug_assert!(self.contains(row));
        Point::new(
            x,
            u16::try_from(row - self.first).expect("visible row fits the terminal viewport"),
        )
    }

    pub(super) const fn next_state(self) -> TranscriptViewState {
        TranscriptViewState {
            mode: self.mode,
            first_visible_row: self.first,
        }
    }
}

fn away_from_tail(
    current: usize,
    amount: usize,
    current_mode: TranscriptViewMode,
) -> (usize, TranscriptViewMode) {
    let next = current.saturating_sub(amount);
    if next == current {
        (next, current_mode)
    } else {
        (next, TranscriptViewMode::Detached)
    }
}

fn toward_tail(current: usize, amount: usize, maximum_first: usize) -> (usize, TranscriptViewMode) {
    let next = current.saturating_add(amount).min(maximum_first);
    if next == maximum_first {
        (next, TranscriptViewMode::FollowTail)
    } else {
        (next, TranscriptViewMode::Detached)
    }
}
