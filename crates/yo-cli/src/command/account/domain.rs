use yo_core::{AccountCapacitySnapshot, AccountId, KimiCatalogSeed, ProviderId};

use super::qwencloud;
use crate::AppError;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct AccountCoordinate {
    pub(super) provider: ProviderId,
    pub(super) account: AccountId,
}

impl AccountCoordinate {
    pub(super) fn new(provider: &str, account: &str) -> Result<Self, AppError> {
        Ok(Self {
            provider: ProviderId::new(provider)
                .map_err(|error| AppError::single("resolving account Provider", error))?,
            account: AccountId::new(account)
                .map_err(|error| AppError::single("resolving account Account", error))?,
        })
    }

    pub(super) fn reference(&self) -> String {
        format!("{}:{}", self.provider, self.account)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum AccountTarget {
    Exact(AccountCoordinate),
    LocalHost { provider: ProviderId },
}

impl AccountTarget {
    pub(super) fn provider(&self) -> &ProviderId {
        match self {
            Self::Exact(coordinate) => &coordinate.provider,
            Self::LocalHost { provider } => provider,
        }
    }

    pub(super) fn exact_coordinate(&self) -> Option<&AccountCoordinate> {
        match self {
            Self::Exact(coordinate) => Some(coordinate),
            Self::LocalHost { .. } => None,
        }
    }

    pub(super) fn reference(&self) -> String {
        match self {
            Self::Exact(coordinate) => coordinate.reference(),
            Self::LocalHost { provider } => local_host_label(provider),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AccountQuery {
    All,
    Provider(ProviderId),
    Exact(AccountCoordinate),
}

#[derive(Clone)]
pub(super) struct AccountCapacityRecord {
    pub(super) target: AccountTarget,
    pub(super) report: Option<AccountCapacityReport>,
}
pub(super) struct AccountRefreshFailure {
    pub(super) target: String,
    pub(super) message: String,
}

#[derive(Clone)]
pub(super) struct AccountCapacityReport {
    pub(super) snapshot: AccountCapacitySnapshot,
    pub(super) provider_data: Option<AccountProviderData>,
    pub(super) observed_at: Option<String>,
}

impl AccountCapacityReport {
    pub(super) fn plain(snapshot: AccountCapacitySnapshot) -> Self {
        Self {
            snapshot,
            provider_data: None,
            observed_at: None,
        }
    }

    pub(super) fn from_cached(
        snapshot: AccountCapacitySnapshot,
        account_label: Option<String>,
        provider_data: Option<AccountProviderData>,
        observed_at: String,
    ) -> Self {
        let snapshot = match account_label {
            Some(label) => snapshot.with_account_label(label),
            None => snapshot,
        };
        Self {
            snapshot,
            provider_data,
            observed_at: Some(observed_at),
        }
    }

    pub(super) fn with_observed_at(mut self, observed_at: String) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    #[cfg(test)]
    pub(super) fn with_account_label(mut self, account_label: impl Into<String>) -> Self {
        self.snapshot = self.snapshot.with_account_label(account_label);
        self
    }

    pub(super) fn coordinate(&self) -> AccountCoordinate {
        AccountCoordinate {
            provider: self.snapshot.provider().clone(),
            account: self.snapshot.account().clone(),
        }
    }

    pub(super) fn snapshot(&self) -> &AccountCapacitySnapshot {
        &self.snapshot
    }

    pub(super) fn account_label(&self) -> &str {
        self.snapshot.account_label()
    }

    pub(super) fn provider_data(&self) -> Option<&AccountProviderData> {
        self.provider_data.as_ref()
    }

    pub(super) fn observed_at(&self) -> Option<&str> {
        self.observed_at.as_deref()
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum AccountProviderData {
    QwenCloud(qwencloud::QwenCloudProviderData),
}
pub(super) fn account_selector_matches(
    target: &AccountCoordinate,
    report: &AccountCapacityReport,
) -> bool {
    target.account == *report.snapshot().account()
        || target
            .account
            .as_str()
            .strip_prefix("account-")
            .is_some_and(|legacy| legacy == report.snapshot().account().as_str())
        || report
            .snapshot()
            .account()
            .as_str()
            .strip_prefix("account-")
            .is_some_and(|legacy| legacy == target.account.as_str())
        || target.account.as_str() == report.account_label()
        || target
            .account
            .as_str()
            .eq_ignore_ascii_case(report.account_label())
}

pub(super) fn exact_refresh_matches(
    query: &AccountQuery,
    target: &AccountTarget,
    report: &AccountCapacityReport,
) -> Option<bool> {
    match query {
        AccountQuery::Exact(expected)
            if expected.provider.as_str() == target.provider().as_str()
                && matches!(target.provider().as_str(), "codex" | "grok") =>
        {
            target
                .exact_coordinate()
                .map(|_| account_selector_matches(expected, report))
        },
        _ => None,
    }
}

pub(super) fn cached_report_is_selectable(
    connections: &yo_core::ConnectionSnapshot,
    report: &AccountCapacityReport,
) -> bool {
    match report.snapshot().provider().as_str() {
        "codex" | "grok" => true,
        "kimi" | "qwencloud" => {
            is_stored_capacity_account(connections, &report.coordinate()).unwrap_or(false)
        },
        _ => false,
    }
}

pub(super) fn target_display_reference(
    target: &AccountTarget,
    cached: &[AccountCapacityReport],
) -> String {
    let Some(coordinate) = target.exact_coordinate() else {
        return target.reference();
    };
    cached
        .iter()
        .find(|report| report.coordinate() == *coordinate)
        .map(|report| format!("{}:{}", coordinate.provider, report.account_label()))
        .unwrap_or_else(|| coordinate.reference())
}

pub(super) fn latest_host_cache<'a>(
    cached: &'a [AccountCapacityReport],
    provider: &str,
) -> Option<&'a AccountCapacityReport> {
    cached
        .iter()
        .filter(|report| report.snapshot().provider().as_str() == provider)
        .max_by_key(|report| report.observed_at().unwrap_or_default())
}

pub(super) fn is_stored_capacity_account(
    connections: &yo_core::ConnectionSnapshot,
    target: &AccountCoordinate,
) -> Result<bool, AppError> {
    match target.provider.as_str() {
        "kimi" => Ok(has_kimi_code_membership_binding(connections, target)),
        "qwencloud" => Ok(has_qwencloud_token_plan_binding(
            connections,
            &target.provider,
            &target.account,
        )),
        _ => Ok(false),
    }
}

pub(super) fn has_kimi_code_membership_binding(
    connections: &yo_core::ConnectionSnapshot,
    target: &AccountCoordinate,
) -> bool {
    connections
        .kimi_catalog_seed(&target.provider, &target.account)
        .ok()
        .flatten()
        .is_some_and(|seed| seed.profile().as_str() == "kimi-code-membership/v1")
        || connections.models().iter().any(|stored| {
            let binding = stored.complete().binding();
            binding.provider_id() == &target.provider
                && binding.account_id() == &target.account
                && KimiCatalogSeed::from_code_membership_binding(binding)
                    .ok()
                    .flatten()
                    .is_some()
        })
}

pub(super) fn host_command_available(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(command);
        let Ok(metadata) = std::fs::metadata(candidate) else {
            return false;
        };
        metadata.is_file() && {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    })
}

pub(super) fn has_qwencloud_token_plan_binding(
    connections: &yo_core::ConnectionSnapshot,
    provider: &ProviderId,
    account: &AccountId,
) -> bool {
    connections
        .models()
        .iter()
        .map(|stored| stored.complete().binding())
        .any(|binding| {
            binding.provider_id() == provider
                && binding.account_id() == account
                && binding.endpoint().as_str().trim_end_matches('/')
                    == "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
        })
}

pub(super) fn local_host_label(provider: &ProviderId) -> String {
    format!("Local {}", display_identifier(provider.as_str()))
}

pub(super) fn display_identifier(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let safe = terminal_safe(part);
            let mut characters = safe.chars();
            match characters.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn terminal_safe(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}
