//! Prompt-adjacent overlays split into pure presentation and token-scoped ownership.

mod binding;
pub(crate) mod selection;
mod slot;

pub(crate) use binding::OverlayBindings;
pub(crate) use selection::{
    PanelPaintError, PanelSnapshot, SelectionEntry, SelectionPanel, SelectionPanelAppearance,
    SelectionPanelGlyphs, SelectionPanelStyles,
};
pub(crate) use slot::{
    AcceptanceReceipt, OverlayInputEffect, OverlayInstanceToken, OverlayPresentation,
    PromptOverlaySlot, SlotError,
};

#[cfg(test)]
mod tests;
