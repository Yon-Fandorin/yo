use serde_json::Value;
use yo_core::{CredentialStore, ToolDefinition, ToolSemanticAdmission, ToolSemanticAdmissionError};

pub(crate) struct LocalSemanticAdmission {
    credentials: CredentialStore,
}

impl LocalSemanticAdmission {
    pub(crate) const fn new(credentials: CredentialStore) -> Self {
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
    fn admit_progress(
        &self,
        definition: &ToolDefinition,
        output: &str,
    ) -> Result<Option<String>, ToolSemanticAdmissionError> {
        if definition.effect() != yo_core::ToolEffect::Process {
            return Ok(None);
        }
        let mut value: Value = serde_json::from_str(output)
            .map_err(|_| ToolSemanticAdmissionError::new("invalid command progress"))?;
        let fields = value
            .as_object_mut()
            .filter(|fields| fields.len() == 2)
            .ok_or_else(|| ToolSemanticAdmissionError::new("invalid command progress"))?;
        for field in ["stdout", "stderr"] {
            let text = fields
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| ToolSemanticAdmissionError::new("invalid command progress"))?;
            self.admit(text)?;
            let safe = self
                .credentials
                .without_incomplete_secret_suffix(text)
                .to_owned();
            fields.insert(field.to_owned(), Value::String(safe));
        }
        Ok(Some(value.to_string()))
    }

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
