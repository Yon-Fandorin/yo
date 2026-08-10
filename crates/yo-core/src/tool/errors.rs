use std::fmt;

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
    pub(super) fn new(message: impl Into<String>) -> Self {
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
