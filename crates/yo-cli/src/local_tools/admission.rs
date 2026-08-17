use yo_core::{ToolDefinition, ToolSemanticAdmission, ToolSemanticAdmissionError};

pub(crate) struct LocalSemanticAdmission {
    credentials: yo_core::CredentialStore,
}

impl LocalSemanticAdmission {
    pub(crate) const fn new(credentials: yo_core::CredentialStore) -> Self {
        Self { credentials }
    }

    fn admit(&self, value: &str) -> Result<String, ToolSemanticAdmissionError> {
        if self.credentials.contains_secret_material(value) {
            Err(ToolSemanticAdmissionError::new(
                "tool semantic value contains prohibited credential material",
            ))
        } else {
            Ok(value.to_owned())
        }
    }
}

impl ToolSemanticAdmission for LocalSemanticAdmission {
    fn admit_arguments(
        &self,
        definition: &ToolDefinition,
        validated_argument_bytes: &str,
    ) -> Result<String, ToolSemanticAdmissionError> {
        let admitted = self.admit(validated_argument_bytes)?;
        let arguments = serde_json::from_str(&admitted).map_err(|_| {
            ToolSemanticAdmissionError::new("validated tool arguments are not JSON")
        })?;
        super::filesystem::validate_arguments(definition, &arguments)
            .map_err(|error| ToolSemanticAdmissionError::new(error.to_string()))?;
        Ok(admitted)
    }

    fn admit_output(
        &self,
        _definition: &ToolDefinition,
        bounded_output: &str,
    ) -> Result<String, ToolSemanticAdmissionError> {
        self.admit(bounded_output)
    }
}
