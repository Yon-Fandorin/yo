use yo_core::{ModelSelection, ModelSelectionController};

use crate::overlay::{PanelSnapshot, SelectionEntry};

#[derive(Clone, Debug)]
pub(super) struct ModelSelectionState {
    controller: ModelSelectionController,
}

impl ModelSelectionState {
    pub(super) const fn new(controller: ModelSelectionController) -> Self {
        Self { controller }
    }

    pub(super) fn panel(&self) -> Result<PanelSnapshot, String> {
        let entries = self
            .controller
            .choices()
            .iter()
            .map(|choice| {
                let selection = choice.selection();
                SelectionEntry::enabled_with_context(
                    selection.row_identity(),
                    choice.model_label(),
                    Some(format!(
                        "{} › {}",
                        choice.provider_label(),
                        choice.account_label()
                    )),
                    Some(format!(
                        "{} / {} / {}",
                        selection.provider(),
                        selection.account(),
                        selection.model()
                    )),
                )
            })
            .collect();
        PanelSnapshot::new("Select model", entries)
            .map_err(|error| format!("the configured model catalog cannot be displayed: {error:?}"))
    }

    pub(super) fn resolve_direct(&self, value: &str) -> Result<ModelSelection, String> {
        self.controller
            .resolve_reference(value)
            .map_err(|error| error.to_string())
    }

    pub(super) fn accept_identity(&self, identity: &str) -> Result<ModelSelection, String> {
        self.controller
            .accept_row_identity(identity)
            .map_err(|error| error.to_string())
    }

    pub(super) fn is_current(&self, selection: &ModelSelection) -> bool {
        self.controller.current() == Some(selection)
    }
}
