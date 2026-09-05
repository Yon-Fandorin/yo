//! Prompt-local built-in command composition and public crate facade.

mod compact;
mod definition;
mod exit;
mod help;
mod model;
mod palette;
mod registry;

pub(crate) use definition::{CommandDefinition, CommandEffect, CommandId};
pub(crate) use palette::CommandPalette;
pub(crate) use registry::CommandRegistry;

pub(crate) fn model_argument(value: &str) -> Option<&str> {
    model::argument(value)
}

pub(crate) fn compact_argument(value: &str) -> Option<&str> {
    compact::argument(value)
}
