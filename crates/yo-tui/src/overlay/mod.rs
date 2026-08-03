//! Prompt-adjacent overlays split into pure presentation and token-scoped ownership.

mod binding;
pub(crate) mod selection;
mod slot;

pub(crate) use binding::OverlayBindings;
#[cfg(test)]
pub(crate) use selection::SelectionEntry;
pub(crate) use selection::{
    PanelPaintError, PanelSnapshot, SelectionPanel, SelectionPanelAppearance, SelectionPanelGlyphs,
    SelectionPanelStyles,
};
pub(crate) use slot::{
    AcceptanceReceipt, OverlayInputEffect, OverlayInstanceToken, PromptOverlaySlot, SlotError,
};

#[cfg(test)]
mod tests;
