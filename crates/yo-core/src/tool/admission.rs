use std::fmt;

use super::schema::ToolDefinition;

pub trait ToolSemanticAdmission: Send {
    /// Admits a nonterminal snapshot only when the policy supports incomplete output.
    /// None withholds progress; existing policies remain final-output-only by default.
    fn admit_progress(
        &self,
        _definition: &ToolDefinition,
        _bounded_output: &str,
    ) -> Result<Option<String>, ToolSemanticAdmissionError> {
        Ok(None)
    }

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

impl fmt::Display for ToolSemanticAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ToolSemanticAdmissionError {}
