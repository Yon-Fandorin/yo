//! Structural command-tool configuration; artifact access belongs to execution tools.

use std::path::{Component, Path};

use serde::{Deserialize, Deserializer, de::Error as DeserializeError};
use serde_json::Value;
use yo_core::{
    TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolId, ToolRegistry,
};
use yo_yaml::Error as YamlError;

use super::ConfigError;

// Frozen BasicFiles identities in agent.tool.local-execution-boundary. Runtime
// assembly also checks the actual built-in-plus-command registry for collisions.
const RESERVED_IDS: [&str; 5] = [
    "list-files",
    "read-files",
    "edit-file",
    "write-file",
    "run-command",
];
const RESERVED_NAMES: [&str; 5] = [
    "list_files",
    "read_files",
    "edit_file",
    "write_file",
    "run_command",
];
const MAX_COMMANDS: usize = 16;
const MAX_LOCATOR_BYTES: usize = 4096;
const MAX_ARGUMENT_ENTRIES: usize = 32;
const MAX_FIXED_ARGUMENT_BYTES: usize = 16384;
const MAX_CALL_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;
const INVALID_TOOLS_STRUCTURE: &str =
    "tools.commands configuration has an invalid closed structure";

pub(super) fn deserialize_tools<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ToolsConfig, D::Error> {
    present(deserializer).map_err(|_| D::Error::custom(INVALID_TOOLS_STRUCTURE))
}

pub(super) fn is_tools_decode_error(error: &YamlError) -> bool {
    matches!(error.without_snippet(), YamlError::Message { msg, .. } if msg == INVALID_TOOLS_STRUCTURE)
}

#[derive(Clone, Debug)]
pub(crate) struct CommandToolConfig {
    definition: ToolDefinition,
    executable: String,
    script: Option<String>,
    executable_args: Vec<String>,
    argv: Vec<String>,
}

impl CommandToolConfig {
    pub(crate) fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    pub(crate) fn executable(&self) -> &str {
        &self.executable
    }

    pub(crate) fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    pub(crate) fn executable_args(&self) -> &[String] {
        &self.executable_args
    }

    pub(crate) fn argv(&self) -> &[String] {
        &self.argv
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToolsConfig {
    #[serde(default, deserialize_with = "present")]
    commands: Vec<CommandConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandConfig {
    #[serde(deserialize_with = "present")]
    id: String,
    #[serde(deserialize_with = "present")]
    name: String,
    #[serde(deserialize_with = "present")]
    description: String,
    #[serde(deserialize_with = "present")]
    executable: String,
    #[serde(deserialize_with = "present")]
    parameters: Value,
    #[serde(default, deserialize_with = "present_script")]
    script: Option<String>,
    #[serde(default, deserialize_with = "present")]
    executable_args: Vec<String>,
    #[serde(default, deserialize_with = "present")]
    argv: Vec<String>,
}

// The YAML backend accepts null as an empty struct or sequence. Presence is a
// separate command-config rule, so detect null before decoding those types.
fn present<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer)?
        .ok_or_else(|| D::Error::custom("command configuration fields cannot be null"))
}

fn present_script<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    present(deserializer).map(Some)
}

impl ToolsConfig {
    pub(super) fn into_commands(self, path: &Path) -> Result<Vec<CommandToolConfig>, ConfigError> {
        let invalid = |detail| ConfigError::InvalidTools {
            path: path.to_owned(),
            detail,
        };
        if self.commands.len() > MAX_COMMANDS {
            return Err(invalid(
                "at most 16 explicitly configured commands are supported",
            ));
        }
        let mut commands = Vec::with_capacity(self.commands.len());
        for command in self.commands {
            if RESERVED_IDS.contains(&command.id.as_str())
                || RESERVED_NAMES.contains(&command.name.as_str())
            {
                return Err(invalid("command identity collides with a built-in tool"));
            }
            if !valid_locator(&command.executable) || !Path::new(&command.executable).is_absolute()
            {
                return Err(invalid(
                    "executable must be an absolute, nonempty path of at most 4096 UTF-8 bytes without controls",
                ));
            }
            if let Some(script) = &command.script
                && (!valid_locator(script) || !relative_path_stays_beneath(script))
            {
                return Err(invalid(
                    "script must be a nonempty path of at most 4096 UTF-8 bytes without controls; relative paths must stay beneath the Session workspace",
                ));
            }
            let arguments = command.executable_args.iter().chain(&command.argv);
            if command
                .executable_args
                .len()
                .saturating_add(command.argv.len())
                > MAX_ARGUMENT_ENTRIES
                || arguments
                    .clone()
                    .any(|value| value.len() > MAX_LOCATOR_BYTES || value.contains('\0'))
                || arguments.map(String::len).sum::<usize>() > MAX_FIXED_ARGUMENT_BYTES
            {
                return Err(invalid(
                    "fixed argument arrays share at most 32 entries and 16384 UTF-8 bytes; each entry permits at most 4096 bytes and no NUL",
                ));
            }
            let definition = ToolId::new(command.id)
                .and_then(|id| ToolDefinition::new(
                    id,
                    command.name,
                    command.description,
                    TOOL_SCHEMA_DIALECT,
                    command.parameters,
                    ToolEffect::Process,
                    ToolApprovalRequirement::Required,
                ))
                .and_then(|definition| definition.with_argument_byte_limit(MAX_CALL_ARGUMENT_BYTES))
                .map_err(|_| invalid("command identity, description or parameter schema does not satisfy the local tool definition contract"))?;
            commands.push(CommandToolConfig {
                definition,
                executable: command.executable,
                script: command.script,
                executable_args: command.executable_args,
                argv: command.argv,
            });
        }
        ToolRegistry::new(commands.iter().map(|command| command.definition.clone()))
            .map_err(|_| invalid("command IDs and names must each be unique"))?;
        Ok(commands)
    }
}

fn valid_locator(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_LOCATOR_BYTES && !value.chars().any(char::is_control)
}

fn relative_path_stays_beneath(value: &str) -> bool {
    let path = Path::new(value);
    if path.is_absolute() {
        return true;
    }
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {},
            Component::ParentDir => match depth.checked_sub(1) {
                Some(parent) => depth = parent,
                None => return false,
            },
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests;
