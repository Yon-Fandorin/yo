//! Frontend-neutral registry, validation, approval binding, and execution-host ports.
//!
//! The implementation is kept in cohesive private modules while this facade
//! preserves the crate's established tool surface and test paths.

mod admission;
mod approval;
mod errors;
mod execution;
mod registry;
mod schema;

pub use admission::{ToolSemanticAdmission, ToolSemanticAdmissionError};
pub use approval::ToolApprovalBinding;
pub use errors::{
    ToolExecutionError, ToolRegistryError, ToolValidationError, ToolValidationFailure,
};
pub use execution::{
    ToolExecution, ToolExecutionHost, ToolExecutionOutcome, ToolExecutionPoll,
    ToolExecutionRequest, ToolExecutionResult,
};
pub use registry::{FrozenToolRegistry, ToolRegistry, ValidatedToolCall};
// Keep the characterized unit-test paths attached to the facade's private
// constants, as they were before the implementation split.
#[cfg(test)]
use schema::{MAX_DESCRIPTION_BYTES, MAX_ID_BYTES, MAX_SCHEMA_BYTES};
pub use schema::{
    TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolId,
};

#[cfg(test)]
mod tests;
