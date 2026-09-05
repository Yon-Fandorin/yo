//! Fullscreen frame trust and absolute cursor placement.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the fullscreen presenter lands before its application loop consumer"
    )
)]

mod renderer;

use std::{error::Error, fmt};

pub(crate) use renderer::{FullscreenRenderError, FullscreenRenderer};

use crate::surface::{FrameDiff, Point, Size, Surface};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FullscreenFramePlan {
    Complete { current: Size, cursor: Point },
    Update { current: Size, cursor: Point },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FullscreenFrameError {
    CursorOutOfBounds { cursor: Point, size: Size },
    CurrentSizeMismatch { expected: Size, actual: Size },
    PreviousFrameRequired,
    PreviousSizeMismatch { expected: Size, actual: Size },
}

impl fmt::Display for FullscreenFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CursorOutOfBounds { cursor, size } => write!(
                formatter,
                "fullscreen cursor {},{} is outside {}x{}",
                cursor.x, cursor.y, size.width, size.height
            ),
            Self::CurrentSizeMismatch { expected, actual } => write!(
                formatter,
                "current fullscreen frame is {}x{}, expected {}x{}",
                actual.width, actual.height, expected.width, expected.height
            ),
            Self::PreviousFrameRequired => formatter
                .write_str("a trusted fullscreen update requires the previous completed frame"),
            Self::PreviousSizeMismatch { expected, actual } => write!(
                formatter,
                "previous fullscreen frame is {}x{}, expected {}x{}",
                actual.width, actual.height, expected.width, expected.height
            ),
        }
    }
}

impl Error for FullscreenFrameError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FrameTrust {
    #[default]
    Empty,
    Valid(Size),
    Untrusted,
}

#[derive(Debug, Default)]
pub(crate) struct FullscreenViewport {
    trust: FrameTrust,
}

impl FullscreenViewport {
    pub(crate) fn begin_frame(
        &mut self,
        current: Size,
        cursor: Point,
    ) -> Result<PendingFullscreenFrame<'_>, FullscreenFrameError> {
        if cursor.x >= current.width || cursor.y >= current.height {
            return Err(FullscreenFrameError::CursorOutOfBounds {
                cursor,
                size: current,
            });
        }

        let plan = match self.trust {
            FrameTrust::Valid(previous) if previous == current => {
                FullscreenFramePlan::Update { current, cursor }
            },
            FrameTrust::Empty | FrameTrust::Valid(_) | FrameTrust::Untrusted => {
                FullscreenFramePlan::Complete { current, cursor }
            },
        };
        self.trust = FrameTrust::Untrusted;

        Ok(PendingFullscreenFrame {
            viewport: self,
            plan,
            committed: false,
        })
    }

    pub(crate) fn invalidate_frame(&mut self) {
        self.trust = FrameTrust::Untrusted;
    }
}

#[derive(Debug)]
pub(crate) struct PendingFullscreenFrame<'viewport> {
    viewport: &'viewport mut FullscreenViewport,
    plan: FullscreenFramePlan,
    committed: bool,
}

impl PendingFullscreenFrame<'_> {
    pub(crate) const fn plan(&self) -> FullscreenFramePlan {
        self.plan
    }

    pub(crate) const fn cursor(&self) -> Point {
        match self.plan {
            FullscreenFramePlan::Complete { cursor, .. }
            | FullscreenFramePlan::Update { cursor, .. } => cursor,
        }
    }

    pub(crate) fn diff<'current>(
        &self,
        previous: Option<&Surface>,
        current: &'current Surface,
    ) -> Result<FrameDiff<'current>, FullscreenFrameError> {
        let expected = current_size(self.plan);
        if current.size() != expected {
            return Err(FullscreenFrameError::CurrentSizeMismatch {
                expected,
                actual: current.size(),
            });
        }

        match self.plan {
            FullscreenFramePlan::Complete { .. } => {
                Ok(FrameDiff::complete(current.size(), current))
            },
            FullscreenFramePlan::Update { .. } => {
                let previous = previous.ok_or(FullscreenFrameError::PreviousFrameRequired)?;
                if previous.size() != expected {
                    return Err(FullscreenFrameError::PreviousSizeMismatch {
                        expected,
                        actual: previous.size(),
                    });
                }
                Ok(FrameDiff::between(previous, current))
            },
        }
    }

    pub(crate) fn commit(mut self) {
        self.viewport.trust = FrameTrust::Valid(current_size(self.plan));
        self.committed = true;
    }
}

impl Drop for PendingFullscreenFrame<'_> {
    fn drop(&mut self) {
        if !self.committed {
            debug_assert_eq!(self.viewport.trust, FrameTrust::Untrusted);
        }
    }
}

const fn current_size(plan: FullscreenFramePlan) -> Size {
    match plan {
        FullscreenFramePlan::Complete { current, .. }
        | FullscreenFramePlan::Update { current, .. } => current,
    }
}

#[cfg(test)]
mod tests;
