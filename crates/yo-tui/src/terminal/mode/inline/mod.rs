use crate::surface::Size;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InlineFramePlan {
    Initialize {
        current: Size,
    },
    Update {
        current: Size,
    },
    Reconcile {
        previous: Size,
        current: Size,
        owned_rows: u16,
    },
    Reanchor {
        abandoned_rows: u16,
        current: Size,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ViewportState {
    #[default]
    Empty,
    Owned {
        size: Size,
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
        let plan = match self.state {
            ViewportState::Empty => InlineFramePlan::Initialize { current },
            ViewportState::Owned {
                size: previous,
                frame_valid: true,
            } if previous == current => InlineFramePlan::Update { current },
            ViewportState::Owned { size: previous, .. } => InlineFramePlan::Reconcile {
                previous,
                current,
                owned_rows: previous.height.max(current.height),
            },
            ViewportState::Abandoned { rows } => InlineFramePlan::Reanchor {
                abandoned_rows: rows,
                current,
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

        PendingFrame {
            viewport: self,
            plan,
            committed: false,
        }
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

    pub(crate) fn commit(mut self) {
        let current = match self.plan {
            InlineFramePlan::Initialize { current }
            | InlineFramePlan::Update { current }
            | InlineFramePlan::Reconcile { current, .. }
            | InlineFramePlan::Reanchor { current, .. } => current,
        };
        self.viewport.state = ViewportState::Owned {
            size: current,
            frame_valid: true,
        };
        self.committed = true;
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

#[cfg(test)]
mod tests;
