//! Cursor-range and physical-scroll evidence for one publication transaction.

use super::super::InlineFramePlan;
use crate::{surface::Size, terminal::TerminalOp};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScrollEvidence {
    None,
    Possible,
    Definite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorRange {
    minimum: u16,
    maximum: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EffectEvidence {
    cursor_before: CursorRange,
    cursor_after: CursorRange,
    pub(super) scroll: ScrollEvidence,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Ledger {
    cursor: CursorRange,
    bottom: u16,
}

impl Ledger {
    pub(super) fn new(plan: InlineFramePlan, terminal_size: Size) -> Self {
        let bottom = terminal_size.height.saturating_sub(1);
        let cursor = initial_cursor_range(plan, bottom);
        Self { cursor, bottom }
    }

    pub(super) fn observe(&mut self, operations: &[TerminalOp<'_>]) -> EffectEvidence {
        let cursor_before = self.cursor;
        let mut scroll = ScrollEvidence::None;
        for operation in operations {
            scroll = combine_scroll(scroll, self.apply(*operation));
        }
        EffectEvidence {
            cursor_before,
            cursor_after: self.cursor,
            scroll,
        }
    }

    fn apply(&mut self, operation: TerminalOp<'_>) -> ScrollEvidence {
        match operation {
            TerminalOp::MoveTo(point) => {
                self.cursor = CursorRange {
                    minimum: point.y.min(self.bottom),
                    maximum: point.y.min(self.bottom),
                };
            },
            TerminalOp::MoveUp { rows } => {
                self.cursor.minimum = self.cursor.minimum.saturating_sub(rows.get());
                self.cursor.maximum = self.cursor.maximum.saturating_sub(rows.get());
            },
            TerminalOp::MoveDown { rows } => {
                self.cursor.minimum = self
                    .cursor
                    .minimum
                    .saturating_add(rows.get())
                    .min(self.bottom);
                self.cursor.maximum = self
                    .cursor
                    .maximum
                    .saturating_add(rows.get())
                    .min(self.bottom);
            },
            TerminalOp::LineFeed => {
                let scroll = if self.cursor.minimum == self.bottom {
                    ScrollEvidence::Definite
                } else if self.cursor.maximum == self.bottom {
                    ScrollEvidence::Possible
                } else {
                    ScrollEvidence::None
                };
                self.cursor.minimum = self.cursor.minimum.saturating_add(1).min(self.bottom);
                self.cursor.maximum = self.cursor.maximum.saturating_add(1).min(self.bottom);
                return scroll;
            },
            TerminalOp::FrameSizeChanged { .. }
            | TerminalOp::SetStyle(_)
            | TerminalOp::WriteGrapheme { .. }
            | TerminalOp::WriteBlank { .. }
            | TerminalOp::SetCursorVisible(_)
            | TerminalOp::MoveToColumn(_)
            | TerminalOp::CarriageReturn
            | TerminalOp::EraseLine
            | TerminalOp::EraseToLineEnd => {},
        }
        ScrollEvidence::None
    }
}

fn initial_cursor_range(plan: InlineFramePlan, bottom: u16) -> CursorRange {
    let owned = match plan {
        InlineFramePlan::Update {
            current,
            previous_cursor,
            ..
        } => Some((current.height, previous_cursor.y)),
        InlineFramePlan::Reconcile {
            previous,
            previous_cursor,
            ..
        } => Some((previous.height, previous_cursor.y)),
        InlineFramePlan::Initialize { .. } | InlineFramePlan::Reanchor { .. } => None,
    };
    owned.map_or(
        CursorRange {
            minimum: 0,
            maximum: bottom,
        },
        |(height, relative)| CursorRange {
            minimum: relative.min(bottom),
            maximum: bottom
                .saturating_sub(height)
                .saturating_add(relative)
                .min(bottom),
        },
    )
}

fn combine_scroll(left: ScrollEvidence, right: ScrollEvidence) -> ScrollEvidence {
    match (left, right) {
        (ScrollEvidence::Definite, _) | (_, ScrollEvidence::Definite) => ScrollEvidence::Definite,
        (ScrollEvidence::Possible, _) | (_, ScrollEvidence::Possible) => ScrollEvidence::Possible,
        (ScrollEvidence::None, ScrollEvidence::None) => ScrollEvidence::None,
    }
}
