use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, ProviderId,
};

use super::{LEGACY_SCHEMA, MAX_CACHE_BYTES, MAX_ENTRIES, SCHEMA, StorageError};
use crate::command::account::domain::{AccountCapacityReport, AccountProviderData};

pub(super) fn decode(
    path: &Path,
    encoded: &[u8],
) -> Result<Vec<AccountCapacityReport>, StorageError> {
    let file: WireCacheFile =
        yo_yaml::from_slice(encoded).map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
    if !matches!(file.schema.as_str(), SCHEMA | LEGACY_SCHEMA) || file.entries.len() > MAX_ENTRIES {
        return Err(StorageError::InvalidContents(path.to_owned()));
    }
    let mut reports = Vec::with_capacity(file.entries.len());
    for entry in file.entries {
        let report = decode_entry(path, entry)?;
        if reports
            .iter()
            .any(|known: &AccountCapacityReport| known.coordinate() == report.coordinate())
        {
            return Err(StorageError::InvalidContents(path.to_owned()));
        }
        reports.push(report);
    }
    Ok(reports)
}

pub(super) fn encode(reports: &[AccountCapacityReport]) -> Result<Vec<u8>, StorageError> {
    if reports.len() > MAX_ENTRIES {
        return Err(StorageError::InvalidContents(PathBuf::new()));
    }
    let entries = reports
        .iter()
        .map(WireCacheEntry::from_report)
        .collect::<Result<Vec<_>, _>>()?;
    let encoded = yo_yaml::to_string(&WireCacheFile {
        schema: SCHEMA.to_owned(),
        entries,
    })
    .map(String::into_bytes)
    .map_err(|_| StorageError::InvalidContents(PathBuf::new()))?;
    if encoded.len() as u64 > MAX_CACHE_BYTES {
        return Err(StorageError::TooLarge(PathBuf::new()));
    }
    Ok(encoded)
}

fn decode_entry(path: &Path, entry: WireCacheEntry) -> Result<AccountCapacityReport, StorageError> {
    if let Some(label) = entry.account_label.as_deref() {
        AccountId::new(label.to_owned())
            .map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
    }
    let provider = ProviderId::new(entry.snapshot.provider)
        .map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
    let account = AccountId::new(entry.snapshot.account)
        .map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
    let buckets = entry
        .snapshot
        .buckets
        .into_iter()
        .map(|bucket| decode_bucket(path, bucket))
        .collect::<Result<Vec<_>, _>>()?;
    let observed_at = validate_timestamp(path, &entry.observed_at)?;
    Ok(AccountCapacityReport::from_cached(
        AccountCapacitySnapshot::new(provider, account, buckets),
        entry.account_label,
        entry.provider_data,
        observed_at,
    ))
}

fn decode_bucket(path: &Path, bucket: WireBucket) -> Result<AccountCapacityBucket, StorageError> {
    let primary = bucket
        .primary
        .map(|window| decode_window(path, window))
        .transpose()?;
    let secondary = bucket
        .secondary
        .map(|window| decode_window(path, window))
        .transpose()?;
    Ok(AccountCapacityBucket::new(
        bucket.id,
        bucket.name,
        bucket.plan,
        primary,
        secondary,
        bucket.credits.map(|credits| {
            AccountCredits::new(credits.balance, credits.has_credits, credits.unlimited)
        }),
        bucket.limit_reason,
    ))
}

fn decode_window(path: &Path, window: WireWindow) -> Result<AccountCapacityWindow, StorageError> {
    let decoded = match (window.reported_used, window.reported_limit) {
        (Some(used), Some(limit)) => AccountCapacityWindow::from_usage_ratio(
            used,
            limit,
            window.window_duration_minutes,
            window.resets_at_unix_seconds,
        ),
        (None, None) => AccountCapacityWindow::from_used_percent_basis_points(
            window.used_percent_basis_points,
            window.window_duration_minutes,
            window.resets_at_unix_seconds,
        ),
        _ => return Err(StorageError::InvalidContents(path.to_owned())),
    }
    .map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
    if decoded.used_percent_basis_points() != window.used_percent_basis_points {
        return Err(StorageError::InvalidContents(path.to_owned()));
    }
    Ok(decoded)
}

fn validate_timestamp(path: &Path, value: &str) -> Result<String, StorageError> {
    let timestamp = value
        .parse::<jiff::Timestamp>()
        .map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
    if timestamp.subsec_nanosecond() != 0 || timestamp.to_string() != value {
        return Err(StorageError::InvalidContents(path.to_owned()));
    }
    Ok(value.to_owned())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireCacheFile {
    pub(super) schema: String,
    pub(super) entries: Vec<WireCacheEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireCacheEntry {
    observed_at: String,
    snapshot: WireSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) provider_data: Option<AccountProviderData>,
}

impl WireCacheEntry {
    pub(super) fn from_report(report: &AccountCapacityReport) -> Result<Self, StorageError> {
        let observed_at = report
            .observed_at()
            .ok_or_else(|| StorageError::InvalidContents(PathBuf::new()))?;
        let observed_at = validate_timestamp(Path::new(""), observed_at)?;
        let account_label = report.account_label().to_owned();
        AccountId::new(account_label.clone())
            .map_err(|_| StorageError::InvalidContents(PathBuf::new()))?;
        Ok(Self {
            observed_at,
            snapshot: WireSnapshot::from_snapshot(report.snapshot()),
            account_label: Some(account_label),
            provider_data: report.provider_data().cloned(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireSnapshot {
    provider: String,
    account: String,
    buckets: Vec<WireBucket>,
}

impl WireSnapshot {
    fn from_snapshot(snapshot: &AccountCapacitySnapshot) -> Self {
        Self {
            provider: snapshot.provider().as_str().to_owned(),
            account: snapshot.account().as_str().to_owned(),
            buckets: snapshot
                .buckets()
                .iter()
                .map(WireBucket::from_bucket)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireBucket {
    id: Option<String>,
    name: Option<String>,
    plan: Option<String>,
    primary: Option<WireWindow>,
    secondary: Option<WireWindow>,
    credits: Option<WireCredits>,
    limit_reason: Option<String>,
}

impl WireBucket {
    fn from_bucket(bucket: &AccountCapacityBucket) -> Self {
        Self {
            id: bucket.id().map(str::to_owned),
            name: bucket.name().map(str::to_owned),
            plan: bucket.plan().map(str::to_owned),
            primary: bucket.primary().copied().map(WireWindow::from_window),
            secondary: bucket.secondary().copied().map(WireWindow::from_window),
            credits: bucket.credits().map(WireCredits::from_credits),
            limit_reason: bucket.limit_reason().map(str::to_owned),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireWindow {
    used_percent_basis_points: u16,
    reported_used: Option<u64>,
    reported_limit: Option<u64>,
    window_duration_minutes: Option<u64>,
    resets_at_unix_seconds: Option<i64>,
}

impl WireWindow {
    fn from_window(window: AccountCapacityWindow) -> Self {
        let (reported_used, reported_limit) = window
            .reported_usage()
            .map_or((None, None), |(used, limit)| (Some(used), Some(limit)));
        Self {
            used_percent_basis_points: window.used_percent_basis_points(),
            reported_used,
            reported_limit,
            window_duration_minutes: window.window_duration_minutes(),
            resets_at_unix_seconds: window.resets_at_unix_seconds(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireCredits {
    balance: Option<String>,
    has_credits: bool,
    unlimited: bool,
}

impl WireCredits {
    fn from_credits(credits: &AccountCredits) -> Self {
        Self {
            balance: credits.balance().map(str::to_owned),
            has_credits: credits.has_credits(),
            unlimited: credits.unlimited(),
        }
    }
}
