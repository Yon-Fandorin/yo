use sha2::{Digest, Sha256};

use super::super::{AccountId, HostId, ModelId, ModelServiceError, ProviderId};

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
    AccountId::new(hex_prefix(&digest, 16))
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

pub(crate) fn encode_coordinate_segment(value: &str) -> String {
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

fn hex_prefix(bytes: &[u8], count: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(count * 2);
    for byte in bytes.iter().take(count) {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
