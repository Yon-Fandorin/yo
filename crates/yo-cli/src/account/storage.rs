use std::{
    error::Error,
    fmt, fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, ProviderId,
};

use super::{AccountCapacityReport, AccountProviderData};

const SCHEMA: &str = "yo.account-capacity-cache/v1alpha2";
const LEGACY_SCHEMA: &str = "yo.account-capacity-cache/v1alpha1";
const MAX_CACHE_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
const LOCK_FILE: &str = ".account-capacity.lock";

#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

#[derive(Debug)]
pub(super) enum StorageError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidPath(PathBuf),
    UnsupportedFileType(PathBuf),
    WrongOwner(PathBuf),
    InsecurePermissions(PathBuf),
    TooLarge(PathBuf),
    Changed(PathBuf),
    InvalidContents(PathBuf),
    Randomness(String),
}

impl StorageError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no parent directory", path.display())
            },
            Self::UnsupportedFileType(path) => write!(
                formatter,
                "{} is not a regular account-capacity cache file",
                path.display()
            ),
            Self::WrongOwner(path) => write!(
                formatter,
                "{} is not owned by the current effective user",
                path.display()
            ),
            Self::InsecurePermissions(path) => write!(
                formatter,
                "{} must not grant group or other permissions",
                path.display()
            ),
            Self::TooLarge(path) => write!(
                formatter,
                "{} exceeds the {MAX_CACHE_BYTES}-byte account-capacity cache limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its account-capacity cache was being read",
                path.display()
            ),
            Self::InvalidContents(path) if path.as_os_str().is_empty() => {
                formatter.write_str("the account-capacity cache contains invalid contents")
            },
            Self::InvalidContents(path) => write!(
                formatter,
                "{} contains an invalid account-capacity cache",
                path.display()
            ),
            Self::Randomness(message) => {
                write!(
                    formatter,
                    "generating an account-capacity cache name failed: {message}"
                )
            },
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(super) fn load(path: &Path) -> Result<Vec<AccountCapacityReport>, StorageError> {
    let Some(encoded) = read_bytes(path)? else {
        return Ok(Vec::new());
    };
    let file: WireCacheFile = yo_yaml::from_slice(&encoded)
        .map_err(|_| StorageError::InvalidContents(path.to_owned()))?;
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

pub(super) fn upsert(path: &Path, updates: &[AccountCapacityReport]) -> Result<(), StorageError> {
    if updates.is_empty() {
        return Ok(());
    }
    reject_symlink(path)?;
    let (parent, lock) = lock_repository(path)?;
    let result = (|| {
        let mut reports = load(path)?;
        for update in updates {
            let coordinate = (
                update.snapshot().provider().as_str(),
                update.snapshot().account().as_str(),
            );
            if matches!(coordinate.0, "codex" | "grok") {
                reports.retain(|existing| {
                    existing.snapshot().provider().as_str() != coordinate.0
                        || existing.snapshot().account().as_str() == coordinate.1
                });
            }
            if let Some(existing) = reports.iter_mut().find(|existing| {
                existing.snapshot().provider().as_str() == coordinate.0
                    && existing.snapshot().account().as_str() == coordinate.1
            }) {
                *existing = update.clone();
            } else {
                reports.push(update.clone());
            }
        }
        reports.sort_by(|left, right| {
            left.snapshot()
                .provider()
                .as_str()
                .cmp(right.snapshot().provider().as_str())
                .then_with(|| {
                    left.snapshot()
                        .account()
                        .as_str()
                        .cmp(right.snapshot().account().as_str())
                })
        });
        let encoded = encode(&reports)?;
        publish(path, &parent, &encoded)
    })();
    drop(lock);
    result
}

fn encode(reports: &[AccountCapacityReport]) -> Result<Vec<u8>, StorageError> {
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

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, StorageError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StorageError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CACHE_BYTES)).unwrap_or(MAX_CACHE_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| StorageError::io(path, source))?;
    if encoded.len() as u64 > MAX_CACHE_BYTES {
        return Err(StorageError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    if before != after {
        return Err(StorageError::Changed(path.to_owned()));
    }
    Ok(Some(encoded))
}

fn lock_repository(path: &Path) -> Result<(PathBuf, fs::File), StorageError> {
    let parent = prepare_parent(path)?;
    let lock_path = parent.join(LOCK_FILE);
    reject_symlink(&lock_path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|source| StorageError::io(&lock_path, source))?;
    MetadataSnapshot::capture(&lock_path, &file)?.validate(&lock_path)?;
    file.lock()
        .map_err(|source| StorageError::io(&lock_path, source))?;
    Ok((parent, file))
}

fn publish(path: &Path, parent: &Path, encoded: &[u8]) -> Result<(), StorageError> {
    reject_symlink(path)?;
    let (temporary, mut file) = create_temporary(parent)?;
    let publication = (|| {
        file.write_all(encoded)
            .map_err(|source| StorageError::io(&temporary, source))?;
        file.sync_all()
            .map_err(|source| StorageError::io(&temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| StorageError::io(path, source))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| StorageError::io(parent, source))?;
        Ok(())
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication
}

fn prepare_parent(path: &Path) -> Result<PathBuf, StorageError> {
    let parent = path
        .parent()
        .ok_or_else(|| StorageError::InvalidPath(path.to_owned()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent)
        && metadata.file_type().is_symlink()
    {
        return Err(StorageError::UnsupportedFileType(parent.to_owned()));
    }
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|source| StorageError::io(parent, source))?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|source| StorageError::io(parent, source))?;
    }
    validate_parent(parent)?;
    Ok(parent.to_owned())
}

fn validate_parent(parent: &Path) -> Result<(), StorageError> {
    let metadata =
        fs::symlink_metadata(parent).map_err(|source| StorageError::io(parent, source))?;
    if !metadata.file_type().is_dir() {
        return Err(StorageError::UnsupportedFileType(parent.to_owned()));
    }
    let shared_sticky_directory =
        metadata.uid() != rustix::process::geteuid().as_raw() && metadata.mode() & 0o1000 != 0;
    if metadata.uid() != rustix::process::geteuid().as_raw() && !shared_sticky_directory {
        return Err(StorageError::WrongOwner(parent.to_owned()));
    }
    if metadata.mode() & 0o022 != 0 && !shared_sticky_directory {
        return Err(StorageError::InsecurePermissions(parent.to_owned()));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), StorageError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(StorageError::UnsupportedFileType(path.to_owned()));
    }
    Ok(())
}

fn create_temporary(parent: &Path) -> Result<(PathBuf, fs::File), StorageError> {
    for _ in 0..16 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| StorageError::Randomness(error.to_string()))?;
        let mut suffix = String::with_capacity(32);
        for byte in random {
            use fmt::Write as _;
            write!(suffix, "{byte:02x}").expect("formatting into a String cannot fail");
        }
        let temporary = parent.join(format!(".account-capacity.{suffix}.pending"));
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(source) => return Err(StorageError::io(&temporary, source)),
        }
    }
    Err(StorageError::InvalidContents(PathBuf::new()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl MetadataSnapshot {
    fn capture(path: &Path, file: &fs::File) -> Result<Self, StorageError> {
        let metadata = file
            .metadata()
            .map_err(|source| StorageError::io(path, source))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            user: metadata.uid(),
            group: metadata.gid(),
            len: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(StorageError::UnsupportedFileType(path.to_owned()));
        }
        if self.user != rustix::process::geteuid().as_raw() {
            return Err(StorageError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o077 != 0 {
            return Err(StorageError::InsecurePermissions(path.to_owned()));
        }
        if self.len > MAX_CACHE_BYTES {
            return Err(StorageError::TooLarge(path.to_owned()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireCacheFile {
    schema: String,
    entries: Vec<WireCacheEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireCacheEntry {
    observed_at: String,
    snapshot: WireSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_data: Option<AccountProviderData>,
}

impl WireCacheEntry {
    fn from_report(report: &AccountCapacityReport) -> Result<Self, StorageError> {
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    // cache는 snapshot, account label, 관측 시각과 count 기반 window를 저장 후 다시 복원합니다.
    #[test]
    fn cache_round_trips_snapshot_and_observation_time() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-cache-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            vec![AccountCapacityBucket::new(
                Some("weekly".to_owned()),
                None,
                Some("Kimi Code".to_owned()),
                Some(AccountCapacityWindow::from_usage_ratio(1, 3, Some(10_080), None).unwrap()),
                None,
                Some(AccountCredits::new(Some("4".to_owned()), true, false)),
                None,
            )],
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_account_label("kimi-default")
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());

        upsert(&path, std::slice::from_ref(&report)).unwrap();
        let loaded = load(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].snapshot(), report.snapshot());
        assert_eq!(loaded[0].account_label(), "kimi-default");
        assert_eq!(loaded[0].observed_at(), Some("2026-09-03T01:02:03Z"));
        assert_eq!(
            loaded[0].snapshot().buckets()[0]
                .primary()
                .unwrap()
                .reported_usage(),
            Some((1, 3))
        );
        assert!(!fs::metadata(&path).unwrap().permissions().readonly());
        let _ = fs::remove_dir_all(root);
    }

    // cache label은 AccountId와 같은 256-byte 경계를 round-trip하고 그 이상은 저장하지 않습니다.
    #[test]
    fn enforces_the_account_label_boundary_before_cache_publication() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-label-boundary-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        let boundary_path = root.join("account-capacity-boundary.yaml");
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("stable-account").unwrap(),
            Vec::new(),
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_account_label("a".repeat(256))
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());

        upsert(&boundary_path, std::slice::from_ref(&report)).unwrap();
        let loaded = load(&boundary_path).unwrap();
        assert_eq!(loaded[0].account_label().len(), 256);

        let too_long = report.with_account_label("a".repeat(257));
        assert!(matches!(
            upsert(&path, std::slice::from_ref(&too_long)),
            Err(StorageError::InvalidContents(_))
        ));
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    // 동일한 Provider·Account 좌표가 중복되면 마지막 값으로 덮지 않고 cache 전체를 거부합니다.
    #[test]
    fn rejects_duplicate_cache_coordinates_instead_of_last_write_wins() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-duplicate-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();

        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let entry = WireCacheEntry::from_report(&report).unwrap();
        let encoded = yo_yaml::to_string(&WireCacheFile {
            schema: SCHEMA.to_owned(),
            entries: vec![entry.clone(), entry],
        })
        .unwrap();
        fs::write(&path, encoded).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE)).unwrap();

        assert!(matches!(load(&path), Err(StorageError::InvalidContents(_))));
        let _ = fs::remove_dir_all(root);
    }

    // 이전 cache schema에서 account label이 없어도 stable AccountId를 표시 label로 사용해 읽습니다.
    #[test]
    fn reads_the_legacy_cache_shape_without_an_account_label() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-legacy-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();

        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("codex").unwrap(),
            AccountId::new("0123456789abcdef").unwrap(),
            Vec::new(),
        ))
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let mut entry = WireCacheEntry::from_report(&report).unwrap();
        entry.account_label = None;
        entry.provider_data = None;
        let encoded = yo_yaml::to_string(&WireCacheFile {
            schema: LEGACY_SCHEMA.to_owned(),
            entries: vec![entry],
        })
        .unwrap();
        fs::write(&path, encoded).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE)).unwrap();

        let loaded = load(&path).unwrap();

        assert_eq!(loaded[0].account_label(), "0123456789abcdef");
        let _ = fs::remove_dir_all(root);
    }

    // host 계정이 바뀌면 이전 host cache를 남기지 않고 새 identity 하나로 교체합니다.
    #[test]
    fn replaces_old_host_cache_identity_when_a_new_host_is_observed() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-host-switch-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        let old = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("grok").unwrap(),
            AccountId::new("old-host-account").unwrap(),
            Vec::new(),
        ))
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let new = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("grok").unwrap(),
            AccountId::new("new-host-account").unwrap(),
            Vec::new(),
        ))
        .with_observed_at("2026-09-03T01:03:03Z".to_owned());

        upsert(&path, std::slice::from_ref(&old)).unwrap();
        upsert(&path, std::slice::from_ref(&new)).unwrap();

        let loaded = load(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].snapshot().account().as_str(), "new-host-account");
        let _ = fs::remove_dir_all(root);
    }

    // cache 디렉터리에 group 또는 other 쓰기 권한이 있으면 저장하지 않습니다.
    #[test]
    fn refuses_a_group_or_other_writable_state_directory() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-insecure-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        let snapshot = AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        );
        let report = AccountCapacityReport::plain(snapshot)
            .with_observed_at("2026-09-03T01:02:03Z".to_owned());

        assert!(matches!(
            upsert(&path, std::slice::from_ref(&report)),
            Err(StorageError::InsecurePermissions(_))
        ));
        let _ = fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE));
        let _ = fs::remove_dir_all(root);
    }

    // cache 경로가 symlink이면 대상 파일을 따라가지 않고 안전하게 거부합니다.
    #[test]
    fn refuses_a_symlink_cache_without_following_its_target() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-symlink-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        let target = root.join("outside.yaml");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();
        fs::write(&target, b"outside").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        ))
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());

        assert!(matches!(
            upsert(&path, std::slice::from_ref(&report)),
            Err(StorageError::UnsupportedFileType(found)) if found == path
        ));
        assert_eq!(fs::read(&target).unwrap(), b"outside");
        let _ = fs::remove_dir_all(root);
    }

    // cache 경로가 regular file이 아닌 directory이면 읽기 대상으로 사용하지 않습니다.
    #[test]
    fn rejects_a_directory_cache_as_non_regular() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-directory-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();

        assert!(matches!(
            load(&path),
            Err(StorageError::UnsupportedFileType(found)) if found == path
        ));
        let _ = fs::remove_dir_all(root);
    }

    // 기존 cache 파일에 group 또는 other 권한이 있으면 secret-adjacent state로 읽지 않습니다.
    #[test]
    fn rejects_an_insecure_existing_cache_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-capacity-file-mode-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("account-capacity.yaml");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();
        let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            Vec::new(),
        ))
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());
        let entry = WireCacheEntry::from_report(&report).unwrap();
        let encoded = yo_yaml::to_string(&WireCacheFile {
            schema: SCHEMA.to_owned(),
            entries: vec![entry],
        })
        .unwrap();
        fs::write(&path, encoded).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(matches!(
            load(&path),
            Err(StorageError::InsecurePermissions(found)) if found == path
        ));
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE));
        let _ = fs::remove_dir_all(root);
    }
}
