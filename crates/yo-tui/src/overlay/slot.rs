use super::{
    binding::{OverlayAction, OverlayBindings},
    selection::{EntryIdentity, PanelSnapshot, PanelValidationError, SelectionPanel},
};
use crate::input::event::{InputEvent, KeyAction};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct OverlayInstanceToken(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OverlayPresentation {
    token: OverlayInstanceToken,
    revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptanceReceipt {
    token: OverlayInstanceToken,
    identity: EntryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OverlayInputEffect {
    Unhandled,
    Consumed,
    Redraw,
    Dismissed(OverlayInstanceToken),
    AcceptedEmpty(OverlayInstanceToken),
    FilterChanged(usize),
    Accepted(AcceptanceReceipt),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SlotError {
    TokenOverflow,
    PresentationRevisionOverflow,
    StaleToken,
    NoSelection,
    ChatNotVisible,
    AgentInteractionPending,
    InvalidPanel(PanelValidationError),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PromptOverlaySlot {
    generation: u64,
    current: Option<OverlayInstance>,
    bindings: OverlayBindings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OverlayInstance {
    token: OverlayInstanceToken,
    panel: SelectionPanel,
    presentation_revision: u64,
    presented: bool,
    presentation_pending: bool,
    accepts_empty: bool,
}

impl PromptOverlaySlot {
    pub(crate) fn open(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.open_inner(snapshot, false)
    }

    pub(crate) fn open_accepting_empty(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.open_inner(snapshot, true)
    }

    fn open_inner(
        &mut self,
        snapshot: PanelSnapshot,
        accepts_empty: bool,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(SlotError::TokenOverflow)?;
        let token = OverlayInstanceToken(self.generation);
        self.current = Some(OverlayInstance {
            token,
            panel: SelectionPanel::new(snapshot),
            presentation_revision: 0,
            presented: false,
            presentation_pending: false,
            accepts_empty,
        });
        Ok(token)
    }

    pub(crate) fn refresh(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        let current = self.matching_mut(token)?;
        if current.presented || current.presentation_pending {
            current.presentation_revision = current
                .presentation_revision
                .checked_add(1)
                .ok_or(SlotError::PresentationRevisionOverflow)?;
            current.presentation_pending = true;
        }
        current.panel.refresh(snapshot);
        Ok(())
    }

    pub(crate) fn close(&mut self, token: OverlayInstanceToken) -> Result<(), SlotError> {
        self.matching(token)?;
        self.close_current();
        Ok(())
    }

    pub(crate) fn close_current(&mut self) {
        self.current = None;
    }

    pub(crate) fn accept(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<AcceptanceReceipt, SlotError> {
        let identity = {
            let current = self.matching(token)?;
            if !current.panel.is_fresh() {
                return Err(SlotError::NoSelection);
            }
            current
                .panel
                .selected_identity()
                .cloned()
                .ok_or(SlotError::NoSelection)?
        };
        self.close_current();
        Ok(AcceptanceReceipt { token, identity })
    }

    pub(crate) fn handle(&mut self, input: &InputEvent) -> OverlayInputEffect {
        if !self
            .current
            .as_ref()
            .is_some_and(|current| current.presented)
        {
            return OverlayInputEffect::Unhandled;
        }
        let Some(binding) = self.bindings.classify(input) else {
            return OverlayInputEffect::Unhandled;
        };
        match binding.action {
            OverlayAction::Dismiss => {
                if binding.key_action == KeyAction::Press {
                    let token = self.current().token;
                    self.close_current();
                    OverlayInputEffect::Dismissed(token)
                } else {
                    OverlayInputEffect::Consumed
                }
            },
            OverlayAction::Previous
            | OverlayAction::Next
            | OverlayAction::FilterPrevious
            | OverlayAction::FilterNext
            | OverlayAction::Accept
                if self.current().presentation_pending =>
            {
                OverlayInputEffect::Consumed
            },
            OverlayAction::Previous => {
                self.current_mut().panel.previous();
                OverlayInputEffect::Redraw
            },
            OverlayAction::Next => {
                self.current_mut().panel.next();
                OverlayInputEffect::Redraw
            },
            OverlayAction::FilterPrevious if !self.current().panel.is_fresh() => {
                OverlayInputEffect::Consumed
            },
            OverlayAction::FilterPrevious => self.current_mut().panel.previous_filter().map_or(
                OverlayInputEffect::Unhandled,
                OverlayInputEffect::FilterChanged,
            ),
            OverlayAction::FilterNext if !self.current().panel.is_fresh() => {
                OverlayInputEffect::Consumed
            },
            OverlayAction::FilterNext => self.current_mut().panel.next_filter().map_or(
                OverlayInputEffect::Unhandled,
                OverlayInputEffect::FilterChanged,
            ),
            OverlayAction::Accept => {
                if binding.key_action == KeyAction::Press {
                    let token = self.current().token;
                    if !self.current().panel.is_fresh() {
                        return OverlayInputEffect::Consumed;
                    }
                    if self.current().panel.selected_identity().is_none() {
                        return if self.current().accepts_empty {
                            OverlayInputEffect::AcceptedEmpty(token)
                        } else {
                            OverlayInputEffect::Consumed
                        };
                    }
                    self.accept(token)
                        .map_or(OverlayInputEffect::Consumed, OverlayInputEffect::Accepted)
                } else {
                    OverlayInputEffect::Consumed
                }
            },
            OverlayAction::Interrupt => OverlayInputEffect::Unhandled,
        }
    }

    pub(crate) fn wants_input(&self, input: &InputEvent) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.presented)
            && self.bindings.classify(input).is_some_and(|binding| {
                binding.action != OverlayAction::Interrupt
                    && (!matches!(
                        binding.action,
                        OverlayAction::FilterPrevious | OverlayAction::FilterNext
                    ) || self.current().panel.has_filter_bar())
            })
    }

    pub(crate) fn panel(&self) -> Option<&SelectionPanel> {
        self.current.as_ref().map(|current| &current.panel)
    }

    #[cfg(test)]
    pub(crate) const fn is_open(&self) -> bool {
        self.current.is_some()
    }

    pub(crate) fn is_current(&self, token: OverlayInstanceToken) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.token == token)
    }

    pub(crate) const fn bindings(&self) -> &OverlayBindings {
        &self.bindings
    }

    pub(crate) fn presentation(&self) -> Option<OverlayPresentation> {
        self.current.as_ref().map(|current| OverlayPresentation {
            token: current.token,
            revision: current.presentation_revision,
        })
    }

    pub(crate) fn commit_presentation(
        &mut self,
        presentation: OverlayPresentation,
        visible: bool,
    ) -> bool {
        let Some(current) = self.current.as_mut().filter(|current| {
            current.token == presentation.token
                && current.presentation_revision == presentation.revision
        }) else {
            return false;
        };
        current.presented = visible;
        current.presentation_pending = false;
        true
    }

    #[cfg(test)]
    pub(crate) fn set_presented(&mut self, presented: bool) {
        if let Some(current) = self.current.as_mut() {
            current.presented = presented;
            current.presentation_pending = false;
        }
    }

    pub(crate) fn set_pending(
        &mut self,
        token: OverlayInstanceToken,
        activity_status: impl Into<String>,
    ) -> Result<(), SlotError> {
        let current = self.matching_mut(token)?;
        current
            .panel
            .set_pending_activity(activity_status)
            .map_err(SlotError::InvalidPanel)?;
        Ok(())
    }

    fn matching(&self, token: OverlayInstanceToken) -> Result<&OverlayInstance, SlotError> {
        self.current
            .as_ref()
            .filter(|current| current.token == token)
            .ok_or(SlotError::StaleToken)
    }

    fn matching_mut(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<&mut OverlayInstance, SlotError> {
        self.current
            .as_mut()
            .filter(|current| current.token == token)
            .ok_or(SlotError::StaleToken)
    }

    fn current(&self) -> &OverlayInstance {
        self.current.as_ref().expect("presented slots have a panel")
    }

    fn current_mut(&mut self) -> &mut OverlayInstance {
        self.current.as_mut().expect("presented slots have a panel")
    }
}

impl AcceptanceReceipt {
    pub(crate) const fn token(&self) -> OverlayInstanceToken {
        self.token
    }

    pub(crate) fn identity(&self) -> &str {
        self.identity.as_str()
    }
}
