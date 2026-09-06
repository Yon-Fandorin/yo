mod renderer;
mod transaction;

use std::{error::Error, fmt};

pub(crate) use renderer::{
    InlineRecovery, InlineRenderError, InlineRenderReceipt, InlineRenderer, InlineRestoreOutcome,
};

use crate::surface::{FrameDiff, Point, Size, Surface};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineFramePlan {
    Initialize {
        current: Size,
        cursor: Point,
    },
    Update {
        current: Size,
        previous_cursor: Point,
        cursor: Point,
    },
    Reconcile {
        previous: Size,
        current: Size,
        owned_rows: u16,
        previous_cursor: Point,
        cursor: Point,
    },
    Reanchor {
        abandoned_rows: u16,
        current: Size,
        cursor: Point,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineRestorePlan {
    Nothing,
    ClearOwned { size: Size, cursor: Point },
    LeaveUntrusted { abandoned_rows: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineFrameError {
    CurrentSizeMismatch { expected: Size, actual: Size },
    PreviousFrameRequired,
    PreviousSizeMismatch { expected: Size, actual: Size },
    CursorOutOfBounds { cursor: Point, size: Size },
    PublicationWidthMismatch { expected: u16, actual: u16 },
    TerminalGeometryMismatch { terminal: Size, live: Size },
}

impl fmt::Display for InlineFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentSizeMismatch { expected, actual } => write!(
                formatter,
                "current frame size is {}x{}, expected {}x{}",
                actual.width, actual.height, expected.width, expected.height
            ),
            Self::PreviousFrameRequired => formatter.write_str("previous frame is required"),
            Self::PreviousSizeMismatch { expected, actual } => write!(
                formatter,
                "previous frame size is {}x{}, expected {}x{}",
                actual.width, actual.height, expected.width, expected.height
            ),
            Self::CursorOutOfBounds { cursor, size } => write!(
                formatter,
                "inline cursor {},{} is outside {}x{}",
                cursor.x, cursor.y, size.width, size.height
            ),
            Self::PublicationWidthMismatch { expected, actual } => write!(
                formatter,
                "persistent rows are {actual} columns wide, expected {expected}"
            ),
            Self::TerminalGeometryMismatch { terminal, live } => write!(
                formatter,
                "inline live frame is {}x{}, outside observed terminal {}x{}",
                live.width, live.height, terminal.width, terminal.height
            ),
        }
    }
}

impl Error for InlineFrameError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ViewportState {
    #[default]
    Empty,
    Owned {
        size: Size,
        cursor: Point,
        frame_valid: bool,
    },
    Abandoned {
        rows: u16,
    },
}

#[derive(Debug, Default)]
pub(crate) struct InlineViewport {
    state: ViewportState,
}

impl InlineViewport {
    pub(crate) fn begin_frame(&mut self, current: Size) -> PendingFrame<'_> {
        self.begin_frame_at(current, Point::new(0, current.height))
            .expect("the logical bottom anchor is valid for every viewport")
    }

    pub(crate) fn begin_frame_at(
        &mut self,
        current: Size,
        cursor: Point,
    ) -> Result<PendingFrame<'_>, InlineFrameError> {
        let inside_surface = cursor.y < current.height && cursor.x < current.width;
        let at_bottom_anchor = cursor == Point::new(0, current.height);
        if !inside_surface && !at_bottom_anchor {
            return Err(InlineFrameError::CursorOutOfBounds {
                cursor,
                size: current,
            });
        }

        let plan = match self.state {
            ViewportState::Empty => InlineFramePlan::Initialize { current, cursor },
            ViewportState::Owned {
                size: previous,
                cursor: previous_cursor,
                frame_valid: true,
            } if previous == current => InlineFramePlan::Update {
                current,
                previous_cursor,
                cursor,
            },
            ViewportState::Owned {
                size: previous,
                cursor: previous_cursor,
                ..
            } => InlineFramePlan::Reconcile {
                previous,
                current,
                owned_rows: previous.height.max(current.height),
                previous_cursor,
                cursor,
            },
            ViewportState::Abandoned { rows } => InlineFramePlan::Reanchor {
                abandoned_rows: rows,
                current,
                cursor,
            },
        };

        let uncertain_rows = match self.state {
            ViewportState::Empty => current.height,
            ViewportState::Owned { size, .. } => size.height.max(current.height),
            ViewportState::Abandoned { rows } => rows.max(current.height),
        };
        self.state = ViewportState::Abandoned {
            rows: uncertain_rows,
        };

        Ok(PendingFrame {
            viewport: self,
            plan,
            committed: false,
        })
    }

    pub(crate) fn invalidate_frame(&mut self) {
        if let ViewportState::Owned { frame_valid, .. } = &mut self.state {
            *frame_valid = false;
        }
    }

    pub(crate) fn abandon_anchor(&mut self) {
        if let ViewportState::Owned { size, .. } = self.state {
            self.state = ViewportState::Abandoned { rows: size.height };
        }
    }

    pub(crate) fn begin_restore(&mut self) -> PendingRestore<'_> {
        let plan = match self.state {
            ViewportState::Empty => InlineRestorePlan::Nothing,
            ViewportState::Owned { size, cursor, .. } => {
                InlineRestorePlan::ClearOwned { size, cursor }
            },
            ViewportState::Abandoned { rows } => InlineRestorePlan::LeaveUntrusted {
                abandoned_rows: rows,
            },
        };

        if let InlineRestorePlan::ClearOwned { size, .. } = plan {
            self.state = ViewportState::Abandoned { rows: size.height };
        }

        PendingRestore {
            viewport: self,
            plan,
            committed: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PendingFrame<'viewport> {
    viewport: &'viewport mut InlineViewport,
    plan: InlineFramePlan,
    committed: bool,
}

impl PendingFrame<'_> {
    pub(crate) const fn plan(&self) -> InlineFramePlan {
        self.plan
    }

    pub(crate) fn diff<'current>(
        &self,
        previous: Option<&Surface>,
        current: &'current Surface,
    ) -> Result<FrameDiff<'current>, InlineFrameError> {
        self.validate_surfaces(previous, current)?;
        match self.plan {
            InlineFramePlan::Update { .. } => Ok(FrameDiff::between(
                previous.expect("validated above"),
                current,
            )),
            InlineFramePlan::Reconcile { previous, .. } => {
                Ok(FrameDiff::complete(previous, current))
            },
            InlineFramePlan::Initialize { .. } | InlineFramePlan::Reanchor { .. } => {
                Ok(FrameDiff::complete(current.size(), current))
            },
        }
    }

    pub(super) fn redraw_diff<'current>(
        &self,
        previous: Option<&Surface>,
        current: &'current Surface,
    ) -> Result<FrameDiff<'current>, InlineFrameError> {
        self.validate_surfaces(previous, current)?;
        Ok(FrameDiff::complete(current.size(), current))
    }

    fn validate_surfaces(
        &self,
        previous: Option<&Surface>,
        current: &Surface,
    ) -> Result<(), InlineFrameError> {
        let expected_current = current_size(self.plan);
        if current.size() != expected_current {
            return Err(InlineFrameError::CurrentSizeMismatch {
                expected: expected_current,
                actual: current.size(),
            });
        }

        match self.plan {
            InlineFramePlan::Update {
                current: expected, ..
            }
            | InlineFramePlan::Reconcile {
                previous: expected, ..
            } => {
                let previous = previous.ok_or(InlineFrameError::PreviousFrameRequired)?;
                if previous.size() != expected {
                    return Err(InlineFrameError::PreviousSizeMismatch {
                        expected,
                        actual: previous.size(),
                    });
                }

                Ok(())
            },
            InlineFramePlan::Initialize { .. } | InlineFramePlan::Reanchor { .. } => Ok(()),
        }
    }

    pub(crate) fn commit(mut self) {
        let current = current_size(self.plan);
        self.viewport.state = ViewportState::Owned {
            size: current,
            cursor: target_cursor(self.plan),
            frame_valid: true,
        };
        self.committed = true;
    }
}

const fn current_size(plan: InlineFramePlan) -> Size {
    match plan {
        InlineFramePlan::Initialize { current, .. }
        | InlineFramePlan::Update { current, .. }
        | InlineFramePlan::Reconcile { current, .. }
        | InlineFramePlan::Reanchor { current, .. } => current,
    }
}

const fn target_cursor(plan: InlineFramePlan) -> Point {
    match plan {
        InlineFramePlan::Initialize { cursor, .. }
        | InlineFramePlan::Update { cursor, .. }
        | InlineFramePlan::Reconcile { cursor, .. }
        | InlineFramePlan::Reanchor { cursor, .. } => cursor,
    }
}

impl Drop for PendingFrame<'_> {
    fn drop(&mut self) {
        if !self.committed {
            debug_assert!(matches!(
                self.viewport.state,
                ViewportState::Abandoned { .. }
            ));
        }
    }
}

#[derive(Debug)]
pub(crate) struct PendingRestore<'viewport> {
    viewport: &'viewport mut InlineViewport,
    plan: InlineRestorePlan,
    committed: bool,
}

impl PendingRestore<'_> {
    pub(crate) const fn plan(&self) -> InlineRestorePlan {
        self.plan
    }

    pub(crate) fn commit(mut self) {
        self.viewport.state = ViewportState::Empty;
        self.committed = true;
    }
}

impl Drop for PendingRestore<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        debug_assert!(matches!(
            (self.plan, self.viewport.state),
            (InlineRestorePlan::Nothing, ViewportState::Empty)
                | (
                    InlineRestorePlan::ClearOwned { .. } | InlineRestorePlan::LeaveUntrusted { .. },
                    ViewportState::Abandoned { .. }
                )
        ));
    }
}

#[cfg(test)]
mod tests;
