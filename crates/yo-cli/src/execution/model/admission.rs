//! Exact native binding admission selected by the process composition root.

use yo_core::{
    AdmittedCompleteBinding, ApiDialect, CompleteModelBinding, ConnectorId, ModelBindingAdmission,
};

pub(crate) struct NativeBindingAdmission;

impl ModelBindingAdmission for NativeBindingAdmission {
    fn admit(&self, complete: &CompleteModelBinding) -> Result<AdmittedCompleteBinding, String> {
        let binding = complete.binding();
        match (binding.connector_id().as_str(), binding.api_dialect()) {
            (ConnectorId::OPENAI_RESPONSES, ApiDialect::OpenAiResponses)
            | (ConnectorId::OPENAI_CHAT_COMPLETIONS, ApiDialect::OpenAiChatCompletions) => {
                yo_core::admit_standard_complete_binding(complete)
            },
            (ConnectorId::KIMI_CHAT_COMPLETIONS, ApiDialect::KimiChatCompletions) => {
                yo_connector_kimi::admit_complete_binding(complete)
                    .map_err(|error| error.to_string())
            },
            _ => Err(format!(
                "unsupported Connector identity {} for API dialect {}",
                binding.connector_id(),
                binding.api_dialect().as_str()
            )),
        }
    }
}

#[cfg(test)]
mod tests;
