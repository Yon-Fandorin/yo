use super::{
    binding::{OverlayAction, OverlayBindings},
    selection::{EntryIdentity, PanelSnapshot, SelectionPanel},
};
use crate::input::event::{InputEvent, KeyAction};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct OverlayInstanceToken(u64);

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
    Accepted(AcceptanceReceipt),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SlotError {
    TokenOverflow,
    StaleToken,
    NoSelection,
    ChatNotVisible,
    AgentInteractionPending,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PromptOverlaySlot {
    generation: u64,
    current: Option<OverlayInstance>,
    presented: bool,
    bindings: OverlayBindings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OverlayInstance {
    token: OverlayInstanceToken,
    panel: SelectionPanel,
    acceptance_enabled: bool,
}

impl PromptOverlaySlot {
    pub(crate) fn open(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(SlotError::TokenOverflow)?;
        let token = OverlayInstanceToken(self.generation);
        self.current = Some(OverlayInstance {
            token,
            panel: SelectionPanel::new(snapshot),
            acceptance_enabled: true,
        });
        self.presented = false;
        Ok(token)
    }

    pub(crate) fn refresh(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        let current = self.matching_mut(token)?;
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
        self.presented = false;
    }

    pub(crate) fn accept(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<AcceptanceReceipt, SlotError> {
        let identity = {
            let current = self.matching(token)?;
            if !current.acceptance_enabled {
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
        if !self.presented {
            return OverlayInputEffect::Unhandled;
        }
        let Some(binding) = self.bindings.classify(input) else {
            return OverlayInputEffect::Unhandled;
        };
        match binding.action {
            OverlayAction::Dismiss => {
                if binding.key_action == KeyAction::Press {
                    self.close_current();
                    OverlayInputEffect::Redraw
                } else {
                    OverlayInputEffect::Consumed
                }
            },
            OverlayAction::Previous => {
                self.current_mut().panel.previous();
                OverlayInputEffect::Redraw
            },
            OverlayAction::Next => {
                self.current_mut().panel.next();
                OverlayInputEffect::Redraw
            },
            OverlayAction::Accept => {
                if binding.key_action == KeyAction::Press {
                    let token = self.current().token;
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
        self.presented
            && self
                .bindings
                .classify(input)
                .is_some_and(|binding| binding.action != OverlayAction::Interrupt)
    }

    pub(crate) fn panel(&self) -> Option<&SelectionPanel> {
        self.current.as_ref().map(|current| &current.panel)
    }

    pub(crate) const fn is_open(&self) -> bool {
        self.current.is_some()
    }

    pub(crate) const fn bindings(&self) -> &OverlayBindings {
        &self.bindings
    }

    pub(crate) fn set_presented(&mut self, presented: bool) {
        self.presented = presented && self.current.is_some();
    }

    pub(crate) fn set_acceptance_enabled(
        &mut self,
        token: OverlayInstanceToken,
        enabled: bool,
    ) -> Result<(), SlotError> {
        self.matching_mut(token)?.acceptance_enabled = enabled;
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
