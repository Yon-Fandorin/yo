mod coordinates;
mod host_catalog;
mod picker;
mod resolve;

pub(super) use coordinates::encode_coordinate_segment;
pub use coordinates::{
    HostModelSelection, ModelPickerTarget, ModelSelection, derive_host_account_id,
    derive_host_catalog_revision,
};
pub use host_catalog::{HostCatalogModel, HostModelCatalog};
pub use picker::{
    ModelPickerChoice, ModelPickerSection, ModelSelectionChoice, ModelSelectionController,
};

fn validate_picker_text(label: &str, value: &str) -> Result<(), super::ModelServiceError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(super::ModelServiceError::new(format!(
            "{label} must contain 1 to 4096 non-control bytes"
        )));
    }
    Ok(())
}
