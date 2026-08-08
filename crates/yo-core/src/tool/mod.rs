//! Frontend-neutral registry, validation, approval binding, and execution-host ports.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{FunctionTool, ModelReplayTool, TurnRef};

const MAX_ID_BYTES: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;
const MAX_SCHEMA_BYTES: usize = 64 * 1024;

/// Closed JSON-schema subset accepted for local tool argument admission.
pub const TOOL_SCHEMA_DIALECT: &str = "yo.tool-schema/v1";

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ToolId(String);

impl ToolId {
    pub fn new(value: impl Into<String>) -> Result<Self, ToolRegistryError> {
        let value = value.into();
        validate_wire_name(&value, "ToolId")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolEffect {
    ReadOnly,
    WorkspaceWrite,
    Process,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolApprovalRequirement {
    Automatic,
    Required,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    id: ToolId,
    wire_name: String,
    description: String,
    schema_version: String,
    input_schema: Value,
    effect: ToolEffect,
    approval: ToolApprovalRequirement,
}

impl ToolDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ToolId,
        wire_name: impl Into<String>,
        description: impl Into<String>,
        schema_version: impl Into<String>,
        input_schema: Value,
        effect: ToolEffect,
        approval: ToolApprovalRequirement,
    ) -> Result<Self, ToolRegistryError> {
        let wire_name = wire_name.into();
        validate_wire_name(&wire_name, "tool wire name")?;
        let description = description.into();
        if description.is_empty()
            || description.len() > MAX_DESCRIPTION_BYTES
            || description.chars().any(char::is_control)
        {
            return Err(ToolRegistryError::new(
                "tool description must be non-empty, bounded, and free of control characters",
            ));
        }
        let schema_version = schema_version.into();
        if schema_version != TOOL_SCHEMA_DIALECT {
            return Err(ToolRegistryError::new(format!(
                "unsupported tool schema version; expected {TOOL_SCHEMA_DIALECT}"
            )));
        }
        let encoded_schema = serde_json::to_vec(&input_schema)
            .map_err(|_| ToolRegistryError::new("tool input schema cannot be encoded"))?;
        if encoded_schema.len() > MAX_SCHEMA_BYTES {
            return Err(ToolRegistryError::new(
                "tool input schema exceeds its byte limit",
            ));
        }
        validate_schema_definition(&input_schema)?;
        Ok(Self {
            id,
            wire_name,
            description,
            schema_version,
            input_schema,
            effect,
            approval,
        })
    }

    pub const fn id(&self) -> &ToolId {
        &self.id
    }

    pub fn wire_name(&self) -> &str {
        &self.wire_name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    pub const fn effect(&self) -> ToolEffect {
        self.effect
    }

    pub const fn approval(&self) -> ToolApprovalRequirement {
        self.approval
    }
}

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

pub trait ToolSemanticAdmission: Send {
    fn admit_arguments(
        &self,
        definition: &ToolDefinition,
        validated_argument_bytes: &str,
    ) -> Result<String, ToolSemanticAdmissionError>;

    fn admit_output(
        &self,
        definition: &ToolDefinition,
        bounded_output: &str,
    ) -> Result<String, ToolSemanticAdmissionError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolSemanticAdmissionError {
    message: String,
}

impl ToolSemanticAdmissionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ToolSemanticAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ToolSemanticAdmissionError {}

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolApprovalBinding {
    turn: TurnRef,
    call_id: String,
    tool_id: ToolId,
    argument_digest: [u8; 32],
    effect: ToolEffect,
    execution_host: String,
}

impl ToolApprovalBinding {
    pub fn new(turn: TurnRef, call: &ValidatedToolCall, execution_host: impl Into<String>) -> Self {
        Self {
            turn,
            call_id: call.call_id.clone(),
            tool_id: call.definition.id.clone(),
            argument_digest: Sha256::digest(&call.normalized_arguments).into(),
            effect: call.definition.effect,
            execution_host: execution_host.into(),
        }
    }

    pub const fn turn(&self) -> TurnRef {
        self.turn
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub const fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    pub const fn effect(&self) -> ToolEffect {
        self.effect
    }

    pub fn execution_host(&self) -> &str {
        &self.execution_host
    }

    pub fn argument_digest_hex(&self) -> String {
        self.argument_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn matches(&self, turn: TurnRef, call: &ValidatedToolCall, host: &str) -> bool {
        self.turn == turn
            && self.call_id == call.call_id
            && self.tool_id == call.definition.id
            && self.argument_digest == Sha256::digest(&call.normalized_arguments).as_slice()
            && self.effect == call.definition.effect
            && self.execution_host == host
    }
}

#[derive(Clone, Debug)]
pub struct ToolExecutionRequest {
    pub turn: TurnRef,
    pub call: ValidatedToolCall,
    pub maximum_output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionOutcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionResult {
    outcome: ToolExecutionOutcome,
    output: String,
    truncated: bool,
}

impl ToolExecutionResult {
    pub fn new(outcome: ToolExecutionOutcome, output: impl Into<String>, truncated: bool) -> Self {
        Self {
            outcome,
            output: output.into(),
            truncated,
        }
    }

    pub const fn outcome(&self) -> ToolExecutionOutcome {
        self.outcome
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionPoll {
    Pending,
    Ready,
}

pub trait ToolExecution: Send {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError>;
    fn take_result(&mut self) -> Option<ToolExecutionResult>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), ToolExecutionError>;
}

pub trait ToolExecutionHost: Send {
    fn identity(&self) -> &str;
    fn is_available(&self, tool: &ToolId) -> bool;
    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError>;
    fn shutdown(&mut self) -> Result<(), ToolExecutionError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolValidationFailure {
    InvalidIdentity,
    ArgumentLimit,
    InvalidJson,
    SchemaMismatch,
    UnknownTool,
    DuplicateIdentity,
    Unavailable,
    ApprovalMismatch,
    SemanticAdmission,
}

impl ToolValidationFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "yo.tool.validation.invalid-identity/v1",
            Self::ArgumentLimit => "yo.tool.validation.argument-limit/v1",
            Self::InvalidJson => "yo.tool.validation.invalid-json/v1",
            Self::SchemaMismatch => "yo.tool.validation.schema-mismatch/v1",
            Self::UnknownTool => "yo.tool.validation.unknown-tool/v1",
            Self::DuplicateIdentity => "yo.tool.validation.duplicate-identity/v1",
            Self::Unavailable => "yo.tool.validation.unavailable/v1",
            Self::ApprovalMismatch => "yo.tool.validation.approval-mismatch/v1",
            Self::SemanticAdmission => "yo.tool.validation.semantic-admission/v1",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolValidationError {
    kind: ToolValidationFailure,
    message: String,
}

impl ToolValidationError {
    pub fn new(kind: ToolValidationFailure, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub const fn kind(&self) -> ToolValidationFailure {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ToolValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ToolValidationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolRegistryError(String);

impl ToolRegistryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ToolRegistryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionError(String);

impl ToolExecutionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ToolExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ToolExecutionError {}

fn validate_wire_name(value: &str, label: &str) -> Result<(), ToolRegistryError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(ToolRegistryError::new(format!(
            "{label} must be a bounded ASCII identifier"
        )))
    } else {
        Ok(())
    }
}

fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), normalize_json(value)))
                    .collect(),
            )
        },
        Value::Array(values) => Value::Array(values.iter().map(normalize_json).collect()),
        _ => value.clone(),
    }
}

fn validate_schema_definition(schema: &Value) -> Result<(), ToolRegistryError> {
    let object = schema
        .as_object()
        .ok_or_else(|| ToolRegistryError::new("tool input schema must be an object"))?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(ToolRegistryError::new(
            "tool input schema root type must be object",
        ));
    }
    validate_schema_node(schema, 0)
}

fn validate_schema_node(schema: &Value, depth: usize) -> Result<(), ToolRegistryError> {
    if depth > 16 {
        return Err(ToolRegistryError::new(
            "tool input schema is nested too deeply",
        ));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| ToolRegistryError::new("schema node must be an object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "type"
                | "description"
                | "properties"
                | "required"
                | "additionalProperties"
                | "items"
                | "enum"
        ) {
            return Err(ToolRegistryError::new(format!(
                "unsupported tool schema keyword `{key}`"
            )));
        }
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolRegistryError::new("every schema node requires a string type"))?;
    if !matches!(
        kind,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    ) {
        return Err(ToolRegistryError::new("unsupported tool schema type"));
    }
    if let Some(description) = object.get("description") {
        let description = description
            .as_str()
            .ok_or_else(|| ToolRegistryError::new("schema description must be a string"))?;
        if description.is_empty()
            || description.len() > MAX_DESCRIPTION_BYTES
            || description.chars().any(char::is_control)
        {
            return Err(ToolRegistryError::new(
                "schema description must be non-empty, bounded, and free of control characters",
            ));
        }
    }
    match kind {
        "object" => {
            let properties = match object.get("properties") {
                Some(properties) => properties
                    .as_object()
                    .ok_or_else(|| ToolRegistryError::new("schema properties must be an object"))?
                    .clone(),
                None => serde_json::Map::new(),
            };
            for (name, child) in &properties {
                if name.is_empty()
                    || name.len() > MAX_ID_BYTES
                    || name.chars().any(char::is_control)
                {
                    return Err(ToolRegistryError::new(
                        "schema property names must be non-empty, bounded, and free of control characters",
                    ));
                }
                validate_schema_node(child, depth + 1)?;
            }
            let mut required_names = HashSet::new();
            if let Some(required) = object.get("required") {
                let required = required
                    .as_array()
                    .ok_or_else(|| ToolRegistryError::new("schema required must be an array"))?;
                for name in required {
                    let name = name.as_str().ok_or_else(|| {
                        ToolRegistryError::new("schema required entries must be strings")
                    })?;
                    if !required_names.insert(name) {
                        return Err(ToolRegistryError::new(
                            "schema required entries must be unique",
                        ));
                    }
                    if !properties.contains_key(name) {
                        return Err(ToolRegistryError::new(
                            "schema required entries must name declared properties",
                        ));
                    }
                }
            }
            if object.get("additionalProperties") != Some(&Value::Bool(false)) {
                return Err(ToolRegistryError::new(
                    "object schemas must set additionalProperties to false",
                ));
            }
            if object.contains_key("items") {
                return Err(ToolRegistryError::new(
                    "object schemas cannot declare array items",
                ));
            }
        },
        "array" => {
            if object.contains_key("properties")
                || object.contains_key("required")
                || object.contains_key("additionalProperties")
            {
                return Err(ToolRegistryError::new(
                    "array schemas cannot declare object-only keywords",
                ));
            }
            let items = object
                .get("items")
                .ok_or_else(|| ToolRegistryError::new("array schemas require items"))?;
            validate_schema_node(items, depth + 1)?;
        },
        _ => {
            if object.contains_key("properties")
                || object.contains_key("required")
                || object.contains_key("additionalProperties")
                || object.contains_key("items")
            {
                return Err(ToolRegistryError::new(
                    "scalar schemas cannot declare object or array keywords",
                ));
            }
        },
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or_else(|| ToolRegistryError::new("schema enum must be an array"))?;
        if values.is_empty() {
            return Err(ToolRegistryError::new("schema enum must not be empty"));
        }
        for (index, value) in values.iter().enumerate() {
            if !instance_matches_type(kind, value) {
                return Err(ToolRegistryError::new(
                    "schema enum values must match the node type",
                ));
            }
            if values[..index].contains(value) {
                return Err(ToolRegistryError::new("schema enum values must be unique"));
            }
        }
    }
    Ok(())
}

fn instance_matches_type(kind: &str, instance: &Value) -> bool {
    match kind {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        _ => false,
    }
}

fn validate_instance(
    schema: &Value,
    instance: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err(format!("{path} exceeds the schema nesting limit"));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{path} schema is not an object"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path} schema has no type"))?;
    let type_matches = instance_matches_type(kind, instance);
    if !type_matches {
        return Err(format!("{path} does not match schema type `{kind}`"));
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array)
        && !values.contains(instance)
    {
        return Err(format!("{path} is not one of the admitted enum values"));
    }
    if let Some(instance) = instance.as_object() {
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !instance.contains_key(name) {
                    return Err(format!("{path}.{name} is required"));
                }
            }
        }
        if object.get("additionalProperties") == Some(&Value::Bool(false)) {
            for name in instance.keys() {
                if !properties.contains_key(name) {
                    return Err(format!("{path}.{name} is not admitted by the schema"));
                }
            }
        }
        for (name, child_schema) in properties {
            if let Some(child) = instance.get(&name) {
                validate_instance(&child_schema, child, &format!("{path}.{name}"), depth + 1)?;
            }
        }
    }
    if let (Some(items), Some(instance)) = (object.get("items"), instance.as_array()) {
        for (index, child) in instance.iter().enumerate() {
            validate_instance(items, child, &format!("{path}[{index}]"), depth + 1)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
