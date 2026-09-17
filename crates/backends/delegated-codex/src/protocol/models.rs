//! Codex model/list catalog와 image modality decoding.
//!
//! 모델 순서, hidden/default marker, pagination cursor와 modality evidence를
//! wire 순서 그대로 보존합니다.

use serde_json::Value;
use yo_core::BackendFailure;

use super::bounds::{protocol_failure, valid_catalog_text};

#[derive(Debug)]
pub(crate) struct ModelListPage {
    pub(crate) models: Vec<ModelListModel>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelListModel {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) hidden: bool,
    pub(crate) is_default: bool,
    pub(crate) image_modality: ModelImageModality,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelImageModality {
    Missing,
    Invalid,
    Explicit { image: bool },
}

/// Codex model/list page를 catalog owner가 소비할 수 있는 typed page로 decode합니다.
pub(crate) fn decode_model_list(result: Value) -> Result<ModelListPage, BackendFailure> {
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_failure("invalid Codex model/list response: missing `data`"))?;
    let mut models = Vec::new();
    for entry in data {
        let id = entry.get("model").and_then(Value::as_str).ok_or_else(|| {
            protocol_failure("invalid Codex model/list response: model has no `model`")
        })?;
        let label = entry
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(id);
        if !valid_catalog_text(id) || !valid_catalog_text(label) {
            return Err(protocol_failure(
                "invalid Codex model/list response: invalid model id or display name",
            ));
        }
        let image_modality = match entry.get("inputModalities") {
            None => ModelImageModality::Missing,
            Some(Value::Array(values)) => {
                let mut text = false;
                let mut image = false;
                let mut valid = true;
                for value in values {
                    match value.as_str() {
                        Some("text") if !text => text = true,
                        Some("image") if !image => image = true,
                        _ => valid = false,
                    }
                }
                if valid {
                    ModelImageModality::Explicit { image }
                } else {
                    ModelImageModality::Invalid
                }
            },
            Some(_) => ModelImageModality::Invalid,
        };
        models.push(ModelListModel {
            id: id.to_owned(),
            label: label.to_owned(),
            hidden: entry
                .get("hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_default: entry
                .get("isDefault")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            image_modality,
        });
    }
    let next_cursor = match result.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) if valid_catalog_text(cursor) => Some(cursor.clone()),
        _ => {
            return Err(protocol_failure(
                "invalid Codex model/list response: invalid nextCursor",
            ));
        },
    };
    Ok(ModelListPage {
        models,
        next_cursor,
    })
}
