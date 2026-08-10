use std::{collections::HashMap, sync::Arc};

use serde_json::Value;

use super::{
    errors::{ToolRegistryError, ToolValidationError, ToolValidationFailure},
    schema::{MAX_ID_BYTES, ToolDefinition, normalize_json, validate_instance},
};
use crate::{FunctionTool, ModelReplayTool};

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    definitions: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub fn new(
        definitions: impl IntoIterator<Item = ToolDefinition>,
    ) -> Result<Self, ToolRegistryError> {
        let definitions = definitions.into_iter().collect::<Vec<_>>();
        let mut ids = HashMap::new();
        let mut names = HashMap::new();
        for definition in &definitions {
            if ids.insert(definition.id().clone(), ()).is_some() {
                return Err(ToolRegistryError::new("duplicate ToolId in registry"));
            }
            if names
                .insert(definition.wire_name().to_owned(), ())
                .is_some()
            {
                return Err(ToolRegistryError::new(
                    "duplicate tool wire name in registry",
                ));
            }
        }
        Ok(Self { definitions })
    }

    pub fn freeze(&self) -> FrozenToolRegistry {
        FrozenToolRegistry::new(self.definitions.clone())
    }
}

#[derive(Clone, Debug)]
pub struct FrozenToolRegistry {
    definitions: Arc<[ToolDefinition]>,
    by_name: HashMap<String, usize>,
}

impl FrozenToolRegistry {
    fn new(definitions: Vec<ToolDefinition>) -> Self {
        let by_name = definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (definition.wire_name().to_owned(), index))
            .collect();
        Self {
            definitions: definitions.into(),
            by_name,
        }
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    pub fn function_tools(&self) -> Result<Vec<FunctionTool>, ToolRegistryError> {
        self.definitions
            .iter()
            .map(|tool| {
                FunctionTool::new(
                    tool.wire_name(),
                    tool.description(),
                    tool.input_schema().clone(),
                )
                .map_err(|error| ToolRegistryError::new(error.to_string()))
            })
            .collect()
    }

    pub fn replay_tools(&self) -> Vec<ModelReplayTool> {
        self.definitions
            .iter()
            .map(|tool| {
                ModelReplayTool::new(
                    tool.wire_name(),
                    tool.description(),
                    tool.schema_version(),
                    tool.input_schema().clone(),
                )
            })
            .collect()
    }

    pub fn validate_call(
        &self,
        call_id: impl Into<String>,
        wire_name: &str,
        argument_bytes: &str,
        maximum_argument_bytes: usize,
    ) -> Result<ValidatedToolCall, ToolValidationError> {
        let call_id = call_id.into();
        if call_id.is_empty() || call_id.len() > MAX_ID_BYTES {
            return Err(ToolValidationError::new(
                ToolValidationFailure::InvalidIdentity,
                "tool call identity is empty or too long",
            ));
        }
        if argument_bytes.len() > maximum_argument_bytes {
            return Err(ToolValidationError::new(
                ToolValidationFailure::ArgumentLimit,
                "tool call arguments exceed the configured limit",
            ));
        }
        let Some(definition) = self
            .by_name
            .get(wire_name)
            .and_then(|index| self.definitions.get(*index))
        else {
            return Err(ToolValidationError::new(
                ToolValidationFailure::UnknownTool,
                "tool call names an unavailable registry entry",
            ));
        };
        let arguments: Value = serde_json::from_str(argument_bytes).map_err(|_| {
            ToolValidationError::new(
                ToolValidationFailure::InvalidJson,
                "tool call arguments are not valid JSON",
            )
        })?;
        validate_instance(definition.input_schema(), &arguments, "$", 0).map_err(|message| {
            ToolValidationError::new(ToolValidationFailure::SchemaMismatch, message)
        })?;
        let normalized_arguments =
            serde_json::to_vec(&normalize_json(&arguments)).map_err(|_| {
                ToolValidationError::new(
                    ToolValidationFailure::InvalidJson,
                    "validated tool arguments cannot be normalized",
                )
            })?;
        Ok(ValidatedToolCall {
            call_id,
            definition: definition.clone(),
            argument_bytes: argument_bytes.to_owned(),
            arguments,
            normalized_arguments,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedToolCall {
    call_id: String,
    definition: ToolDefinition,
    argument_bytes: String,
    arguments: Value,
    normalized_arguments: Vec<u8>,
}

impl ValidatedToolCall {
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub const fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    pub fn argument_bytes(&self) -> &str {
        &self.argument_bytes
    }

    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }

    pub(super) fn normalized_arguments(&self) -> &[u8] {
        &self.normalized_arguments
    }
}
