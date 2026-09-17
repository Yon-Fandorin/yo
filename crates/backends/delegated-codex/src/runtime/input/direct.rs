use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    BackendFailure, BackendFailureKind, ImageInputCapability, InputImageHistory, ModelInputPart,
    UserInput,
};

use super::super::state::Backend;
use crate::{observation, protocol};

pub(super) fn validate_direct_input<P: JsonMessagePeer>(
    backend: &mut Backend<P>,
    input: &UserInput,
) -> Result<(), BackendFailure> {
    if input.images().is_empty() && backend.input_image_history == InputImageHistory::TextOnly {
        return Ok(());
    }
    let Some(model) = backend.selected_model.clone() else {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "Codex image input has no verified selected model",
        ));
    };
    backend.require_image_capability(&model)?;
    if let ImageInputCapability::Supported {
        maximum_occurrences,
        maximum_image_bytes,
        maximum_input_bytes,
    } = backend.image_capability
        && (!input.images().is_empty())
    {
        let total = input.images().iter().try_fold(0_u64, |total, image| {
            total.checked_add(image.snapshot().png().len() as u64)
        });
        if input.images().len() as u64 > u64::from(maximum_occurrences)
            || input
                .images()
                .iter()
                .any(|image| image.snapshot().png().len() as u64 > maximum_image_bytes)
            || total.is_none_or(|total| total > maximum_input_bytes)
        {
            return Err(BackendFailure::new(
                BackendFailureKind::InputOverBudget,
                "Codex image input exceeds the selected model's admitted limits",
            ));
        }
    }
    Ok(())
}
pub(super) fn require_image_capability<P: JsonMessagePeer>(
    backend: &mut Backend<P>,
    expected_model: &str,
) -> Result<(), BackendFailure> {
    if !backend.image_wire_supported {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "Codex image input requires the reviewed image-capable protocol build",
        ));
    }
    if backend.selected_model.as_deref() != Some(expected_model)
        || backend.image_capability == ImageInputCapability::Unknown
    {
        backend.selected_model = Some(expected_model.to_owned());
        backend.refresh_model_capability(expected_model);
    }
    match backend.image_capability {
        ImageInputCapability::Supported { .. } => Ok(()),
        ImageInputCapability::Unsupported => Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "the selected Codex model does not advertise image input",
        )),
        ImageInputCapability::Unknown => Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "Codex image capability could not be established for the selected model",
        )),
    }
}
pub(super) fn refresh_model_capability<P: JsonMessagePeer>(backend: &mut Backend<P>, model: &str) {
    backend.image_capability = observation::observe_model_capability(
        &mut backend.client,
        model,
        backend.image_wire_supported,
    );
}
pub(super) fn project_input(input: &UserInput) -> Result<Vec<Value>, BackendFailure> {
    if input.images().is_empty() {
        return Ok(vec![json!({
            "type": "text",
            "text": input.model_input(),
        })]);
    }
    let parts = input.model_parts();
    ModelInputPart::validate_user_parts(&parts).map_err(|detail| {
        protocol::protocol_failure(format!("invalid Codex image input: {detail}"))
    })?;
    Ok(parts
        .into_iter()
        .map(|part| match part {
            ModelInputPart::Text { text } => json!({ "type": "text", "text": text }),
            ModelInputPart::Image { snapshot } => json!({
                "type": "image",
                "url": format!("data:image/png;base64,{}", STANDARD.encode(snapshot.png())),
            }),
        })
        .collect())
}
