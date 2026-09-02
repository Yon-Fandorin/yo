use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{
    AccountId, HostId, ModelCatalog, ModelId, ModelLastFailure, ModelServiceError, ProviderId,
    StartupTarget,
};

/// One exact model coordinate advertised by an authenticated delegated host account.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HostModelSelection {
    host: HostId,
    account: AccountId,
    model: ModelId,
    catalog_revision: String,
}

/// A picker target keeps managed bindings and delegated host selections in disjoint namespaces.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ModelPickerTarget {
    Managed(ModelSelection),
    Host(HostModelSelection),
}

/// One runtime-advertised host model. Hidden models are omitted by the adapter before this seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCatalogModel {
    id: ModelId,
    label: String,
    selectable: bool,
    unavailable_reason: Option<String>,
}

/// Fresh authenticated model inventory for one delegated host account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostModelCatalog {
    host: HostId,
    host_label: String,
    account: AccountId,
    account_label: String,
    revision: String,
    current_model: Option<ModelId>,
    models: Vec<HostCatalogModel>,
}

/// Presentation-neutral account section in the unified model picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelPickerSection {
    identity: String,
    label: String,
    status: Option<String>,
    choices: Vec<ModelPickerChoice>,
}

/// Presentation-neutral selectable row in one account section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelPickerChoice {
    target: ModelPickerTarget,
    label: String,
    detail: String,
    current: bool,
    enabled: bool,
    disabled_reason: Option<String>,
}

/// Derives Yo's stable local host-account key from exact verified account evidence.
/// The evidence values never appear in the returned identifier.
pub fn derive_host_account_id(
    host: &HostId,
    evidence: &[(&str, &str)],
) -> Result<AccountId, ModelServiceError> {
    if evidence.is_empty()
        || evidence
            .iter()
            .any(|(key, value)| key.is_empty() || value.is_empty())
    {
        return Err(ModelServiceError::new(
            "host account identity requires non-empty verified evidence",
        ));
    }
    let mut digest = Sha256::new();
    digest.update((host.as_str().len() as u64).to_be_bytes());
    digest.update(host.as_str().as_bytes());
    for (key, value) in evidence {
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key.as_bytes());
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    AccountId::new(format!("account-{}", hex_prefix(&digest, 16)))
}

/// Binds a picker row to the exact authenticated visible inventory it came from.
#[must_use]
pub fn derive_host_catalog_revision(
    host: &HostId,
    account: &AccountId,
    current_model: Option<&ModelId>,
    models: &[ModelId],
) -> String {
    let mut digest = Sha256::new();
    for value in [host.as_str(), account.as_str()] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    if let Some(current) = current_model {
        digest.update([1]);
        digest.update((current.as_str().len() as u64).to_be_bytes());
        digest.update(current.as_str().as_bytes());
    } else {
        digest.update([0]);
    }
    for model in models {
        digest.update((model.as_str().len() as u64).to_be_bytes());
        digest.update(model.as_str().as_bytes());
    }
    format!("sha256:{}", hex_prefix(&digest.finalize(), 32))
}

/// One exact Provider/Account/Model coordinate selected for a native model binding.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModelSelection {
    provider: ProviderId,
    account: AccountId,
    model: ModelId,
}

impl ModelSelection {
    #[must_use]
    pub const fn new(provider: ProviderId, account: AccountId, model: ModelId) -> Self {
        Self {
            provider,
            account,
            model,
        }
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// Stable row identity containing all three coordinates, independent of display labels.
    #[must_use]
    pub fn row_identity(&self) -> String {
        format!(
            "{}:{}|{}:{}|{}:{}",
            self.provider.as_str().len(),
            self.provider,
            self.account.as_str().len(),
            self.account,
            self.model.as_str().len(),
            self.model
        )
    }

    /// Canonical complete startup reference with Provider and Account separators escaped.
    #[must_use]
    pub fn canonical_reference(&self) -> String {
        format!(
            "{}:{}:{}",
            encode_coordinate_segment(self.provider.as_str()),
            encode_coordinate_segment(self.account.as_str()),
            self.model
        )
    }
}

impl HostModelSelection {
    #[must_use]
    pub fn new(
        host: HostId,
        account: AccountId,
        model: ModelId,
        catalog_revision: impl Into<String>,
    ) -> Self {
        Self {
            host,
            account,
            model,
            catalog_revision: catalog_revision.into(),
        }
    }

    #[must_use]
    pub const fn host(&self) -> &HostId {
        &self.host
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    #[must_use]
    pub fn catalog_revision(&self) -> &str {
        &self.catalog_revision
    }

    #[must_use]
    pub fn row_identity(&self) -> String {
        format!(
            "host:{}:{}|account:{}:{}|model:{}:{}|catalog:{}:{}",
            self.host.as_str().len(),
            self.host.as_str(),
            self.account.as_str().len(),
            self.account,
            self.model.as_str().len(),
            self.model,
            self.catalog_revision.len(),
            self.catalog_revision,
        )
    }
}

impl ModelPickerTarget {
    #[must_use]
    pub const fn managed(&self) -> Option<&ModelSelection> {
        match self {
            Self::Managed(selection) => Some(selection),
            Self::Host(_) => None,
        }
    }

    #[must_use]
    pub const fn host(&self) -> Option<&HostModelSelection> {
        match self {
            Self::Managed(_) => None,
            Self::Host(selection) => Some(selection),
        }
    }

    #[must_use]
    pub fn row_identity(&self) -> String {
        match self {
            Self::Managed(selection) => format!("managed|{}", selection.row_identity()),
            Self::Host(selection) => selection.row_identity(),
        }
    }

    #[must_use]
    pub const fn model(&self) -> &ModelId {
        match self {
            Self::Managed(selection) => selection.model(),
            Self::Host(selection) => selection.model(),
        }
    }

    #[must_use]
    pub fn coordinate_label(&self) -> String {
        match self {
            Self::Managed(selection) => format!(
                "{}::{}::{}",
                selection.provider(),
                selection.account(),
                selection.model()
            ),
            Self::Host(selection) => format!(
                "host:{}::{}::{}",
                selection.host().as_str(),
                selection.account(),
                selection.model()
            ),
        }
    }
}

impl HostCatalogModel {
    pub fn selectable(id: ModelId, label: impl Into<String>) -> Result<Self, ModelServiceError> {
        let label = label.into();
        validate_picker_text("host model label", &label)?;
        Ok(Self {
            id,
            label,
            selectable: true,
            unavailable_reason: None,
        })
    }

    pub fn unavailable(
        id: ModelId,
        label: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let label = label.into();
        let reason = reason.into();
        validate_picker_text("host model label", &label)?;
        validate_picker_text("host model unavailable reason", &reason)?;
        Ok(Self {
            id,
            label,
            selectable: false,
            unavailable_reason: Some(reason),
        })
    }

    #[must_use]
    pub const fn id(&self) -> &ModelId {
        &self.id
    }
}

impl HostModelCatalog {
    pub fn new(
        host: HostId,
        host_label: impl Into<String>,
        account: AccountId,
        account_label: impl Into<String>,
        revision: impl Into<String>,
        current_model: Option<ModelId>,
        models: Vec<HostCatalogModel>,
    ) -> Result<Self, ModelServiceError> {
        let host_label = host_label.into();
        let account_label = account_label.into();
        let revision = revision.into();
        validate_picker_text("host label", &host_label)?;
        validate_picker_text("host account label", &account_label)?;
        validate_picker_text("host catalog revision", &revision)?;
        if models.is_empty() {
            return Err(ModelServiceError::new(
                "a host model catalog must contain at least one visible model",
            ));
        }
        let mut ids = BTreeSet::new();
        for model in &models {
            if !ids.insert(model.id.clone()) {
                return Err(ModelServiceError::new(
                    "a host model catalog contains a duplicate model id",
                ));
            }
        }
        if current_model
            .as_ref()
            .is_some_and(|current| !ids.contains(current))
        {
            return Err(ModelServiceError::new(
                "the current host model is absent from the visible catalog",
            ));
        }
        Ok(Self {
            host,
            host_label,
            account,
            account_label,
            revision,
            current_model,
            models,
        })
    }
}

impl ModelPickerSection {
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    #[must_use]
    pub fn choices(&self) -> &[ModelPickerChoice] {
        &self.choices
    }

    fn is_current(&self) -> bool {
        self.choices.iter().any(ModelPickerChoice::is_current)
    }
}

impl ModelPickerChoice {
    #[must_use]
    pub const fn target(&self) -> &ModelPickerTarget {
        &self.target
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.current
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }
}

/// A presentation-neutral catalog projection in Provider -> Account -> Model order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSelectionChoice {
    selection: ModelSelection,
    provider_label: String,
    account_label: String,
    model_label: String,
    last_failure: Option<ModelLastFailure>,
    enabled: bool,
}

impl ModelSelectionChoice {
    #[must_use]
    pub const fn selection(&self) -> &ModelSelection {
        &self.selection
    }

    #[must_use]
    pub fn provider_label(&self) -> &str {
        &self.provider_label
    }

    #[must_use]
    pub fn account_label(&self) -> &str {
        &self.account_label
    }

    #[must_use]
    pub fn model_label(&self) -> &str {
        &self.model_label
    }

    #[must_use]
    pub const fn last_failure(&self) -> Option<&ModelLastFailure> {
        self.last_failure.as_ref()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn disabled_reason(&self) -> Option<&'static str> {
        if self.enabled {
            None
        } else {
            Some("disabled by operator")
        }
    }
}

/// Owns direct-command resolution and exact picker acceptance outside any frontend.
#[derive(Clone, Debug)]
pub struct ModelSelectionController {
    catalog: ModelCatalog,
    current: Option<ModelSelection>,
    choices: Vec<ModelSelectionChoice>,
    sections: Vec<ModelPickerSection>,
}

impl ModelSelectionController {
    #[must_use]
    pub fn new(catalog: ModelCatalog, current: Option<ModelSelection>) -> Self {
        let mut choices = catalog
            .entries()
            .iter()
            .map(|entry| {
                let binding = entry.binding();
                ModelSelectionChoice {
                    selection: ModelSelection::new(
                        binding.provider_id().clone(),
                        binding.account_id().clone(),
                        binding.model_id().clone(),
                    ),
                    provider_label: entry
                        .provider_display_name()
                        .unwrap_or(binding.provider_id().as_str())
                        .to_owned(),
                    account_label: entry
                        .account_display_name()
                        .unwrap_or(binding.account_id().as_str())
                        .to_owned(),
                    model_label: entry
                        .model_display_name()
                        .unwrap_or(binding.model_id().as_str())
                        .to_owned(),
                    last_failure: entry.last_failure().cloned(),
                    enabled: entry.is_enabled(),
                }
            })
            .collect::<Vec<_>>();
        choices.sort_by(|left, right| left.selection.cmp(&right.selection));
        let mut controller = Self {
            catalog,
            current,
            choices,
            sections: Vec::new(),
        };
        controller.rebuild_managed_sections();
        controller
    }

    #[must_use]
    pub fn choices(&self) -> &[ModelSelectionChoice] {
        &self.choices
    }

    #[must_use]
    pub fn current(&self) -> Option<&ModelSelection> {
        self.current.as_ref()
    }

    /// Adds one fresh host inventory. `active` marks only the live host account's exact current
    /// model; inventories from other available hosts remain ordinary sections.
    pub fn with_host_catalog(mut self, catalog: HostModelCatalog, active: bool) -> Self {
        if active {
            self.current = None;
            for section in &mut self.sections {
                for choice in &mut section.choices {
                    if choice.current {
                        choice.current = false;
                        if let Some(label) = choice.label.strip_suffix(" (current)") {
                            choice.label = label.to_owned();
                        }
                    }
                }
            }
        }
        let section_identity = host_section_identity(&catalog.host, &catalog.account);
        self.sections
            .retain(|section| section.identity != section_identity);
        let choices = catalog
            .models
            .into_iter()
            .map(|model| {
                let current = active && catalog.current_model.as_ref() == Some(&model.id);
                let target = ModelPickerTarget::Host(HostModelSelection::new(
                    catalog.host.clone(),
                    catalog.account.clone(),
                    model.id.clone(),
                    catalog.revision.clone(),
                ));
                ModelPickerChoice {
                    target,
                    label: if current {
                        format!("{} (current)", model.label)
                    } else {
                        model.label
                    },
                    detail: model.id.to_string(),
                    current,
                    enabled: model.selectable,
                    disabled_reason: model.unavailable_reason,
                }
            })
            .collect();
        self.sections.push(ModelPickerSection {
            identity: section_identity,
            label: format!("{} · {}", catalog.host_label, catalog.account_label),
            status: None,
            choices,
        });
        self.sort_sections();
        self
    }

    /// Adds a non-selectable account-local status without suppressing sibling accounts.
    pub fn with_host_status(
        mut self,
        host: &HostId,
        host_label: impl Into<String>,
        account: &AccountId,
        account_label: impl Into<String>,
        status: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let host_label = host_label.into();
        let account_label = account_label.into();
        let status = status.into();
        validate_picker_text("host label", &host_label)?;
        validate_picker_text("host account label", &account_label)?;
        validate_picker_text("host account status", &status)?;
        let identity = host_section_identity(host, account);
        self.sections.retain(|section| section.identity != identity);
        self.sections.push(ModelPickerSection {
            identity,
            label: format!("{host_label} · {account_label}"),
            status: Some(status),
            choices: Vec::new(),
        });
        self.sort_sections();
        Ok(self)
    }

    #[must_use]
    pub fn sections(&self) -> &[ModelPickerSection] {
        &self.sections
    }

    pub fn accept_picker_identity(
        &self,
        identity: &str,
    ) -> Result<ModelPickerTarget, ModelServiceError> {
        let matches = self
            .sections
            .iter()
            .flat_map(ModelPickerSection::choices)
            .filter(|choice| choice.target.row_identity() == identity)
            .collect::<Vec<_>>();
        let [choice] = matches.as_slice() else {
            return Err(ModelServiceError::new(
                "the selected model row is stale or ambiguous",
            ));
        };
        if !choice.enabled {
            return Err(ModelServiceError::new(
                choice
                    .disabled_reason
                    .as_deref()
                    .unwrap_or("the selected model is unavailable"),
            ));
        }
        match &choice.target {
            ModelPickerTarget::Managed(selection) => {
                self.accept_exact(selection).map(ModelPickerTarget::Managed)
            },
            ModelPickerTarget::Host(selection) => Ok(ModelPickerTarget::Host(selection.clone())),
        }
    }

    fn rebuild_managed_sections(&mut self) {
        let mut grouped = BTreeMap::<(String, String, ProviderId, AccountId), Vec<_>>::new();
        for choice in &self.choices {
            grouped
                .entry((
                    choice.provider_label.clone(),
                    choice.account_label.clone(),
                    choice.selection.provider().clone(),
                    choice.selection.account().clone(),
                ))
                .or_default()
                .push(choice);
        }
        self.sections = grouped
            .into_iter()
            .map(
                |((provider_label, account_label, provider, account), choices)| {
                    let choices = choices
                        .into_iter()
                        .map(|choice| {
                            let current = self.current.as_ref() == Some(&choice.selection);
                            ModelPickerChoice {
                                target: ModelPickerTarget::Managed(choice.selection.clone()),
                                label: if current {
                                    format!("{} (current)", choice.model_label)
                                } else {
                                    choice.model_label.clone()
                                },
                                detail: choice.last_failure.as_ref().map_or_else(
                                    || choice.selection.model().to_string(),
                                    |failure| {
                                        format!(
                                            "warning: {} at {}",
                                            failure.kind(),
                                            failure.observed_at()
                                        )
                                    },
                                ),
                                current,
                                enabled: choice.enabled,
                                disabled_reason: choice.disabled_reason().map(str::to_owned),
                            }
                        })
                        .collect();
                    ModelPickerSection {
                        identity: managed_section_identity(&provider, &account),
                        label: format!("{provider_label} · {account_label}"),
                        status: None,
                        choices,
                    }
                },
            )
            .collect();
        self.sort_sections();
    }

    fn sort_sections(&mut self) {
        self.sections.sort_by(|left, right| {
            right
                .is_current()
                .cmp(&left.is_current())
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.identity.cmp(&right.identity))
        });
    }

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
        if let Some(host) = super::HostId::from_reference(reference)? {
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
        if let Some(host) = super::HostId::from_reference(reference)? {
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
            .filter(|choice| choice.selection.row_identity() == identity);
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
        self.accept_exact(&choice.selection)
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

pub(super) fn encode_coordinate_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' => encoded.push_str("%25"),
            ':' => encoded.push_str("%3A"),
            _ => encoded.push(character),
        }
    }
    encoded
}

fn provider_model_reference(selection: &ModelSelection) -> String {
    format!(
        "{}::{}",
        encode_coordinate_segment(selection.provider().as_str()),
        selection.model()
    )
}

fn managed_section_identity(provider: &ProviderId, account: &AccountId) -> String {
    format!(
        "section|managed:{}:{}|account:{}:{}",
        provider.as_str().len(),
        provider,
        account.as_str().len(),
        account,
    )
}

fn host_section_identity(host: &HostId, account: &AccountId) -> String {
    format!(
        "section|host:{}:{}|account:{}:{}",
        host.as_str().len(),
        host.as_str(),
        account.as_str().len(),
        account,
    )
}

fn validate_picker_text(label: &str, value: &str) -> Result<(), ModelServiceError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(ModelServiceError::new(format!(
            "{label} must contain 1 to 4096 non-control bytes"
        )));
    }
    Ok(())
}

fn hex_prefix(bytes: &[u8], count: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(count * 2);
    for byte in bytes.iter().take(count) {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
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
