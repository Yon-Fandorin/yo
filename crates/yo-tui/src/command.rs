//! Prompt-local built-in command composition and public crate facade.

mod attach;
mod changes;
pub(crate) use attach::attachment_argument;
mod compact;
mod definition;
mod exit;
mod fork;
mod help;
mod model;
mod new;
mod output;
mod palette;
mod preview;
mod prompt;
mod registry;
mod resume;
mod tree;

pub(crate) use definition::{CommandDefinition, CommandEffect, CommandId};
pub(crate) use palette::CommandPalette;
pub use prompt::{PromptTemplateError, PromptTemplates};
pub(crate) use registry::CommandRegistry;

pub(crate) fn prompt_argument(value: &str) -> Option<&str> {
    prompt::argument(value)
}

pub(crate) fn model_argument(value: &str) -> Option<&str> {
    model::argument(value)
}

pub(crate) fn compact_argument(value: &str) -> Option<&str> {
    compact::argument(value)
}

pub(crate) fn resume_argument(value: &str) -> Option<&str> {
    resume::argument(value)
}

pub(crate) fn fork_argument(value: &str) -> Option<&str> {
    fork::argument(value)
}

pub(crate) fn tree_argument(value: &str) -> Option<&str> {
    tree::argument(value)
}
