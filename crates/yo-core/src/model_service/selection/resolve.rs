use std::collections::BTreeSet;

use super::{
    super::{HostId, ModelServiceError, StartupTarget},
    ModelSelection, ModelSelectionController,
};

impl ModelSelectionController {
    pub fn resolve_reference(&self, reference: &str) -> Result<ModelSelection, ModelServiceError> {
        match self.resolve_target_reference(reference)? {
            StartupTarget::Model(selection) => Ok(selection),
            StartupTarget::Host(host) => Err(ModelServiceError::new(format!(
                "{} is a HostTarget and is unavailable in a model-only selector",
                host.reference()
            ))),
        }
    }

    /// Resolves one stored model coordinate for an operator activation mutation.
    ///
    /// This is lookup rather than model-work admission, so the current activation state is
    /// intentionally visible to the caller instead of rejecting a disabled binding.
    pub fn resolve_reference_for_activation(
        &self,
        reference: &str,
    ) -> Result<ModelSelection, ModelServiceError> {
        if let Some(host) = HostId::from_reference(reference)? {
            return Err(ModelServiceError::new(format!(
                "{} is a HostTarget and has no stored model activation state",
                host.reference()
            )));
        }
        self.resolve_model_reference(reference, false)
    }

    pub fn resolve_target_reference(
        &self,
        reference: &str,
    ) -> Result<StartupTarget, ModelServiceError> {
        if let Some(host) = HostId::from_reference(reference)? {
            return Ok(StartupTarget::Host(host));
        }
        self.resolve_model_reference(reference, true)
            .map(StartupTarget::Model)
    }

    fn resolve_model_reference(
        &self,
        reference: &str,
        require_enabled: bool,
    ) -> Result<ModelSelection, ModelServiceError> {
        let mut matches = BTreeSet::new();
        for choice in &self.choices {
            let selection = choice.selection();
            let bare_is_applicable = self.current.as_ref().is_none_or(|current| {
                current.provider() == selection.provider()
                    && current.account() == selection.account()
            });
            let provider_model_matches = reference == provider_model_reference(selection);
            let complete_coordinate_matches = reference == selection.canonical_reference();
            if (bare_is_applicable && reference == selection.model().as_str())
                || provider_model_matches
                || complete_coordinate_matches
            {
                matches.insert(selection.clone());
            }
        }

        match matches.len() {
            1 if require_enabled => {
                self.accept_exact(matches.first().expect("one reference match exists"))
            },
            1 => Ok(matches.first().expect("one reference match exists").clone()),
            0 => Err(reference_error(
                reference,
                "is not configured",
                self.choices
                    .iter()
                    .map(|choice| choice.selection().clone())
                    .collect(),
            )),
            _ => Err(reference_error(reference, "is ambiguous", matches)),
        }
    }

    pub fn accept_row_identity(&self, identity: &str) -> Result<ModelSelection, ModelServiceError> {
        let mut matches = self
            .choices
            .iter()
            .filter(|choice| choice.selection().row_identity() == identity);
        let Some(choice) = matches.next() else {
            return Err(ModelServiceError::new(
                "the selected model binding is stale or no longer configured",
            ));
        };
        if matches.next().is_some() {
            return Err(ModelServiceError::new(
                "the selected model binding identity is ambiguous",
            ));
        }
        self.accept_exact(choice.selection())
    }

    pub fn accept_exact(
        &self,
        selection: &ModelSelection,
    ) -> Result<ModelSelection, ModelServiceError> {
        self.catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())?
            .require_enabled()?;
        Ok(selection.clone())
    }
}

fn provider_model_reference(selection: &ModelSelection) -> String {
    format!(
        "{}::{}",
        super::encode_coordinate_segment(selection.provider().as_str()),
        selection.model()
    )
}

fn reference_error(
    reference: &str,
    outcome: &str,
    coordinates: BTreeSet<ModelSelection>,
) -> ModelServiceError {
    const MAX_DIAGNOSTIC_REFERENCE_CHARS: usize = 256;

    let mut chars = reference.chars();
    let displayed = chars
        .by_ref()
        .take(MAX_DIAGNOSTIC_REFERENCE_CHARS)
        .collect::<String>();
    let truncation = if chars.next().is_some() {
        " (truncated)"
    } else {
        ""
    };
    let mut message =
        format!("model reference {displayed:?}{truncation} {outcome}; complete coordinates:");
    if coordinates.is_empty() {
        message.push_str("\n- none configured");
    } else {
        for coordinate in coordinates {
            message.push_str(&format!("\n- {}", coordinate.canonical_reference()));
        }
    }
    ModelServiceError::new(message)
}
