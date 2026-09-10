use std::{collections::BTreeMap, error::Error, fmt};

use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Prompt,
    "command.prompt",
    "/prompt",
    "insert a saved prompt into the editor",
    CommandEffect::InsertPrompt,
);

/// Bounded, literal user prompts. Selection edits a draft without executing it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PromptTemplates {
    entries: BTreeMap<String, String>,
}

/// Invalid prompt template configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptTemplateError {
    /// More than 128 named templates.
    TooManyEntries,
    /// A name is empty, exceeds 64 bytes, or contains characters outside ASCII letters,
    /// digits, underscores and hyphens.
    InvalidName,
    /// A body is empty, exceeds 64 KiB, or contains controls other than TAB, CR and LF.
    InvalidBody,
    /// Combined bodies exceed 1 MiB.
    TotalTooLarge,
}

impl fmt::Display for PromptTemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyEntries => "prompts supports at most 128 templates",
            Self::InvalidName => "prompt names require 1–64 ASCII letters, digits, underscores or hyphens",
            Self::InvalidBody => "prompt bodies require 1–65536 UTF-8 bytes without control characters except TAB, CR and LF",
            Self::TotalTooLarge => "combined prompt bodies exceed 1 MiB",
        })
    }
}

impl Error for PromptTemplateError {}

impl PromptTemplates {
    /// Validates one complete configuration without expanding, trimming or executing text.
    pub fn new(entries: BTreeMap<String, String>) -> Result<Self, PromptTemplateError> {
        if entries.len() > 128 {
            return Err(PromptTemplateError::TooManyEntries);
        }
        let mut total = 0usize;
        for (name, body) in &entries {
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Err(PromptTemplateError::InvalidName);
            }
            if body.is_empty()
                || body.len() > 64 * 1024
                || body
                    .chars()
                    .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
            {
                return Err(PromptTemplateError::InvalidBody);
            }
            total += body.len();
            if total > 1024 * 1024 {
                return Err(PromptTemplateError::TotalTooLarge);
            }
        }
        Ok(Self { entries })
    }

    /// Returns the exact configured body for a case-sensitive name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.get(name).map(String::as_str)
    }

    /// Lists names in stable lexical order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

pub(super) fn argument(value: &str) -> Option<&str> {
    value.strip_prefix("/prompt").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}

#[cfg(test)]
mod tests;
