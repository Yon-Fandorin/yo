use std::{
    collections::HashSet,
    env,
    error::Error,
    ffi::OsString,
    fmt, fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use jiff::{Timestamp, fmt::strtime, tz::TimeZone};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use yo_core::{
    AccountId, EffectiveModelBinding, EffectiveModelProfile, ModelCatalog, ModelCatalogEntry,
    ModelContextProfile, ModelId, ModelProfileLayer, ModelProfileParameters, ModelSelection,
    NormalizedEndpoint, ProviderId, StartupTarget, VersionedProfileId,
};

const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d %H:%M %:z";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_DATE_FORMAT_BYTES: usize = 128;
#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

#[derive(Clone, Debug)]
pub(crate) struct Config {
    date_format: String,
    frame_rate_limit: yo_tui::FrameRateLimit,
    source_path: PathBuf,
    snapshot: ConfigSnapshot,
    model_catalog: ModelCatalog,
    startup_target: Option<StartupTarget>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            date_format: DEFAULT_DATE_FORMAT.to_owned(),
            frame_rate_limit: yo_tui::FrameRateLimit::Fps120,
            source_path: PathBuf::new(),
            snapshot: ConfigSnapshot::absent(),
            model_catalog: ModelCatalog::default(),
            startup_target: None,
        }
    }
}

impl Config {
    pub(crate) fn date_formatter(&self) -> Result<DateFormatter, ConfigError> {
        DateFormatter::new(&self.date_format)
    }

    pub(crate) fn frame_rate_limit(&self) -> yo_tui::FrameRateLimit {
        self.frame_rate_limit
    }

    pub(crate) fn model_catalog(&self) -> &ModelCatalog {
        &self.model_catalog
    }

    pub(crate) fn replace_model_catalog(&mut self, model_catalog: ModelCatalog) {
        self.model_catalog = model_catalog;
    }

    pub(crate) fn startup_target(&self) -> Option<&StartupTarget> {
        self.startup_target.as_ref()
    }

    pub(crate) fn credential_path(&self) -> PathBuf {
        self.source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("credentials.yaml")
    }

    pub(crate) fn connection_path(&self) -> PathBuf {
        self.source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("connections.yaml")
    }

    pub(crate) fn snapshot_digest(&self) -> &str {
        &self.snapshot.digest
    }

    pub(crate) fn verify_unchanged(&self) -> Result<(), ConfigError> {
        let current = capture_snapshot(&self.source_path)?;
        if current == self.snapshot {
            Ok(())
        } else {
            Err(ConfigError::Changed(self.source_path.clone()))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfigSnapshot {
    metadata: Option<ConfigMetadata>,
    bytes: Vec<u8>,
    digest: String,
}

impl ConfigSnapshot {
    fn absent() -> Self {
        Self {
            metadata: None,
            bytes: Vec::new(),
            digest: config_digest(&[]),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfigMetadata {
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

#[derive(Clone, Debug)]
pub(crate) struct DateFormatter {
    format: String,
    timezone: TimeZone,
}

impl DateFormatter {
    fn new(format: &str) -> Result<Self, ConfigError> {
        if format.is_empty() || format.len() > MAX_DATE_FORMAT_BYTES {
            return Err(ConfigError::InvalidDateFormat(
                "session.list.date_format must contain 1 to 128 bytes".to_owned(),
            ));
        }
        if format.chars().any(char::is_control) {
            return Err(ConfigError::InvalidDateFormat(
                "session.list.date_format must not contain control characters".to_owned(),
            ));
        }
        let timezone = TimeZone::system();
        let probe = Timestamp::UNIX_EPOCH.to_zoned(timezone.clone());
        strtime::format(format, &probe).map_err(|error| {
            ConfigError::InvalidDateFormat(format!(
                "session.list.date_format `{format}` is invalid: {error}"
            ))
        })?;
        Ok(Self {
            format: format.to_owned(),
            timezone,
        })
    }

    pub(crate) fn format_unix_millis(&self, millis: u64) -> Result<String, ConfigError> {
        let millis = i64::try_from(millis).map_err(|_| ConfigError::TimestampOutOfRange(millis))?;
        let timestamp = Timestamp::from_millisecond(millis)
            .map_err(|_| ConfigError::TimestampOutOfRange(millis.cast_unsigned()))?;
        strtime::format(&self.format, &timestamp.to_zoned(self.timezone.clone()))
            .map_err(|error| ConfigError::InvalidDateFormat(error.to_string()))
    }
}

#[derive(Debug)]
pub(crate) enum ConfigError {
    Environment(&'static str),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    UnsupportedFileType(PathBuf),
    TooLarge(PathBuf),
    InvalidUtf8 {
        path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    Changed(PathBuf),
    InvalidYaml {
        path: PathBuf,
        source: serde_norway::Error,
    },
    UnsupportedVersion {
        path: PathBuf,
        version: u32,
    },
    InvalidDateFormat(String),
    InvalidMaxFps {
        path: PathBuf,
        value: u16,
    },
    InvalidModel {
        path: PathBuf,
        message: String,
    },
    TimestampOutOfRange(u64),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment(message) => formatter.write_str(message),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::UnsupportedFileType(path) => write!(
                formatter,
                "{} is not a regular configuration file",
                path.display()
            ),
            Self::TooLarge(path) => write!(
                formatter,
                "{} exceeds the {MAX_CONFIG_BYTES}-byte Yo configuration limit",
                path.display()
            ),
            Self::InvalidUtf8 { path, .. } => {
                write!(formatter, "{} is not valid UTF-8", path.display())
            },
            Self::InvalidYaml { path, source } => {
                write!(
                    formatter,
                    "{} contains invalid configuration: {source}",
                    path.display()
                )
            },
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "{} uses unsupported configuration version {version}; expected 1",
                path.display()
            ),
            Self::InvalidDateFormat(message) => formatter.write_str(message),
            Self::InvalidMaxFps { path, value } => write!(
                formatter,
                "{}: tui.max_fps must be 60 or 120, not {value}",
                path.display()
            ),
            Self::InvalidModel { path, message } => {
                write!(formatter, "{}: {message}", path.display())
            },
            Self::TimestampOutOfRange(millis) => {
                write!(
                    formatter,
                    "timestamp {millis}ms is outside the supported date range"
                )
            },
            Self::Changed(path) => write!(
                formatter,
                "{} changed while this command was preparing; retry with the current configuration",
                path.display()
            ),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::InvalidYaml { source, .. } => Some(source),
            Self::Environment(_)
            | Self::UnsupportedFileType(_)
            | Self::TooLarge(_)
            | Self::UnsupportedVersion { .. }
            | Self::Changed(_)
            | Self::InvalidDateFormat(_)
            | Self::InvalidMaxFps { .. }
            | Self::InvalidModel { .. }
            | Self::TimestampOutOfRange(_) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    version: u32,
    #[serde(default)]
    session: SessionConfig,
    #[serde(default)]
    tui: TuiConfig,
    #[serde(default)]
    model: ModelConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionConfig {
    #[serde(default)]
    list: SessionListConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionListConfig {
    date_format: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TuiConfig {
    max_fps: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelConfig {
    startup: Option<StartupTargetConfig>,
    #[serde(default)]
    catalog: Authored<Vec<ModelEntryConfig>>,
    #[serde(default)]
    bindings: Authored<Vec<ModelBindingConfig>>,
}

#[derive(Debug, Default)]
enum Authored<T> {
    #[default]
    Missing,
    Present(T),
}

impl<T> Authored<T> {
    fn into_option(self) -> Option<T> {
        match self {
            Self::Missing => None,
            Self::Present(value) => Some(value),
        }
    }
}

impl<'de, T> Deserialize<'de> for Authored<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
enum StartupTargetConfig {
    Host(String),
    Model {
        provider: String,
        account: String,
        model: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelEntryConfig {
    provider: String,
    provider_display_name: Option<String>,
    account: String,
    account_display_name: Option<String>,
    model: String,
    model_display_name: Option<String>,
    api_dialect: String,
    base_url: String,
    input_token_limit: u64,
    max_output_tokens: u64,
    tokenizer_profile: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelBindingConfig {
    provider: String,
    provider_display_name: Option<String>,
    account: String,
    account_display_name: Option<String>,
    base_url: String,
    #[serde(default)]
    profile: Authored<ModelProfileConfig>,
    models: Vec<ModelBindingEntryConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelBindingEntryConfig {
    model: String,
    model_display_name: Option<String>,
    #[serde(default)]
    profile: Authored<ModelProfileConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelProfileConfig {
    #[serde(default)]
    api_dialect: Authored<String>,
    #[serde(default)]
    tokenizer_profile: Authored<String>,
    #[serde(default)]
    input_token_limit: Authored<u64>,
    #[serde(default)]
    max_output_tokens: Authored<u64>,
    #[serde(default)]
    reasoning_parameters: Authored<ModelProfileParameters>,
    #[serde(default)]
    optional_request_parameters: Authored<ModelProfileParameters>,
    #[serde(default)]
    tool_capability_policy: Authored<String>,
    #[serde(default)]
    verification_profile: Authored<String>,
}

pub(crate) fn load() -> Result<Config, ConfigError> {
    let path = config_path()?;
    load_from(&path)
}

pub(crate) fn selected_path() -> Result<PathBuf, ConfigError> {
    config_path()
}

pub(crate) fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let snapshot = capture_snapshot(path)?;
    if snapshot.metadata.is_none() {
        return Ok(Config {
            source_path: path.to_owned(),
            snapshot,
            ..Config::default()
        });
    }
    let contents =
        String::from_utf8(snapshot.bytes.clone()).map_err(|source| ConfigError::InvalidUtf8 {
            path: path.to_owned(),
            source,
        })?;
    parse_snapshot(path, &contents, snapshot)
}

#[cfg(test)]
fn parse(path: &Path, contents: &str) -> Result<Config, ConfigError> {
    parse_snapshot(
        path,
        contents,
        ConfigSnapshot {
            metadata: None,
            bytes: contents.as_bytes().to_vec(),
            digest: config_digest(contents.as_bytes()),
        },
    )
}

fn parse_snapshot(
    path: &Path,
    contents: &str,
    snapshot: ConfigSnapshot,
) -> Result<Config, ConfigError> {
    yo_core::validate_profile_yaml_number_spellings(contents)
        .map_err(|error| invalid_model(path, error.to_string()))?;
    let decoded: FileConfig =
        serde_norway::from_str(contents).map_err(|source| ConfigError::InvalidYaml {
            path: path.to_owned(),
            source,
        })?;
    if decoded.version != 1 {
        return Err(ConfigError::UnsupportedVersion {
            path: path.to_owned(),
            version: decoded.version,
        });
    }
    let frame_rate_limit = match decoded.tui.max_fps.unwrap_or(120) {
        60 => yo_tui::FrameRateLimit::Fps60,
        120 => yo_tui::FrameRateLimit::Fps120,
        value => {
            return Err(ConfigError::InvalidMaxFps {
                path: path.to_owned(),
                value,
            });
        },
    };
    let ModelConfig {
        startup,
        catalog,
        bindings,
    } = decoded.model;
    if matches!(&catalog, Authored::Present(_)) && matches!(&bindings, Authored::Present(_)) {
        return Err(invalid_model(
            path,
            "model.catalog and model.bindings cannot be authored together",
        ));
    }
    let entries = match (catalog, bindings) {
        (Authored::Missing, Authored::Present(bindings)) => model_binding_entries(path, bindings)?,
        (Authored::Present(catalog), Authored::Missing) => catalog
            .into_iter()
            .map(|entry| model_entry(path, entry))
            .collect::<Result<Vec<_>, _>>()?,
        (Authored::Missing, Authored::Missing) => Vec::new(),
        (Authored::Present(_), Authored::Present(_)) => {
            unreachable!("both authored model collections were rejected")
        },
    };
    let model_catalog = ModelCatalog::new(entries).map_err(|error| ConfigError::InvalidModel {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let startup_target = startup
        .map(|startup| startup_target(path, startup, &model_catalog))
        .transpose()?;
    let config = Config {
        date_format: decoded
            .session
            .list
            .date_format
            .unwrap_or_else(|| DEFAULT_DATE_FORMAT.to_owned()),
        frame_rate_limit,
        source_path: path.to_owned(),
        snapshot,
        model_catalog,
        startup_target,
    };
    config.date_formatter().map_err(|error| match error {
        ConfigError::InvalidDateFormat(message) => {
            ConfigError::InvalidDateFormat(format!("{}: {message}", path.display()))
        },
        other => other,
    })?;
    Ok(config)
}

fn capture_snapshot(path: &Path) -> Result<ConfigSnapshot, ConfigError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConfigSnapshot::absent());
        },
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_owned(),
                source,
            });
        },
    };
    let before = config_metadata(path, &file)?;
    if before.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
        return Err(ConfigError::UnsupportedFileType(path.to_owned()));
    }
    if before.len > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CONFIG_BYTES)).unwrap_or(MAX_CONFIG_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge(path.to_owned()));
    }
    let after = config_metadata(path, &file)?;
    if before != after {
        return Err(ConfigError::Changed(path.to_owned()));
    }
    let digest = config_digest(&bytes);
    Ok(ConfigSnapshot {
        metadata: Some(before),
        bytes,
        digest,
    })
}

fn config_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        use fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    encoded
}

fn config_metadata(path: &Path, file: &fs::File) -> Result<ConfigMetadata, ConfigError> {
    let metadata = file.metadata().map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(ConfigMetadata {
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

fn model_entry(path: &Path, entry: ModelEntryConfig) -> Result<ModelCatalogEntry, ConfigError> {
    let invalid = |error: yo_core::ModelServiceError| ConfigError::InvalidModel {
        path: path.to_owned(),
        message: error.to_string(),
    };
    let api_dialect = entry.api_dialect.parse().map_err(&invalid)?;
    let binding = EffectiveModelBinding::new(
        ProviderId::new(entry.provider).map_err(&invalid)?,
        AccountId::new(entry.account).map_err(&invalid)?,
        ModelId::new(entry.model).map_err(&invalid)?,
        api_dialect,
        NormalizedEndpoint::parse(&entry.base_url).map_err(&invalid)?,
    );
    let context = ModelContextProfile::new(
        entry.input_token_limit,
        entry.max_output_tokens,
        entry.tokenizer_profile,
    )
    .map_err(&invalid)?;
    ModelCatalogEntry::new(
        binding,
        entry.provider_display_name,
        entry.account_display_name,
        entry.model_display_name,
        context,
    )
    .map_err(invalid)
}

fn model_binding_entries(
    path: &Path,
    bindings: Vec<ModelBindingConfig>,
) -> Result<Vec<ModelCatalogEntry>, ConfigError> {
    let mut pairs = HashSet::new();
    let mut entries = Vec::new();
    for binding in bindings {
        let invalid = |error: yo_core::ModelServiceError| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        };
        let provider = ProviderId::new(binding.provider).map_err(&invalid)?;
        let account = AccountId::new(binding.account).map_err(&invalid)?;
        if !pairs.insert((provider.clone(), account.clone())) {
            return Err(invalid_model(
                path,
                format!("model.bindings repeats Provider {provider} and Account {account}"),
            ));
        }
        if binding.models.is_empty() {
            return Err(invalid_model(
                path,
                format!(
                    "model.bindings entry for Provider {provider} and Account {account} requires at least one model"
                ),
            ));
        }
        let endpoint = NormalizedEndpoint::parse(&binding.base_url).map_err(&invalid)?;
        let base_profile = binding
            .profile
            .into_option()
            .map(|profile| model_profile_layer(path, profile))
            .transpose()?;
        for model in binding.models {
            let model_id = ModelId::new(model.model).map_err(&invalid)?;
            let model_profile = model
                .profile
                .into_option()
                .map(|profile| model_profile_layer(path, profile))
                .transpose()?
                .unwrap_or_default();
            let profile = EffectiveModelProfile::resolve(base_profile.as_ref(), &model_profile)
                .map_err(&invalid)?;
            let effective_binding = EffectiveModelBinding::new(
                provider.clone(),
                account.clone(),
                model_id,
                profile.api_dialect(),
                endpoint.clone(),
            );
            entries.push(
                ModelCatalogEntry::with_explicit_profile(
                    effective_binding,
                    binding.provider_display_name.clone(),
                    binding.account_display_name.clone(),
                    model.model_display_name,
                    profile,
                )
                .map_err(&invalid)?,
            );
        }
    }
    Ok(entries)
}

fn model_profile_layer(
    path: &Path,
    profile: ModelProfileConfig,
) -> Result<ModelProfileLayer, ConfigError> {
    let invalid = |error: yo_core::ModelServiceError| ConfigError::InvalidModel {
        path: path.to_owned(),
        message: error.to_string(),
    };
    Ok(ModelProfileLayer::new(
        profile
            .api_dialect
            .into_option()
            .map(|value| value.parse())
            .transpose()
            .map_err(&invalid)?,
        profile
            .tokenizer_profile
            .into_option()
            .map(VersionedProfileId::new)
            .transpose()
            .map_err(&invalid)?,
        profile.input_token_limit.into_option(),
        profile.max_output_tokens.into_option(),
        profile.reasoning_parameters.into_option(),
        profile.optional_request_parameters.into_option(),
        profile
            .tool_capability_policy
            .into_option()
            .map(VersionedProfileId::new)
            .transpose()
            .map_err(&invalid)?,
        profile
            .verification_profile
            .into_option()
            .map(VersionedProfileId::new)
            .transpose()
            .map_err(invalid)?,
    ))
}

fn invalid_model(path: &Path, message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidModel {
        path: path.to_owned(),
        message: message.into(),
    }
}

fn startup_target(
    path: &Path,
    startup: StartupTargetConfig,
    catalog: &ModelCatalog,
) -> Result<StartupTarget, ConfigError> {
    let (provider, account, model) = match startup {
        StartupTargetConfig::Host(reference) => {
            return if reference == StartupTarget::HOST_CODEX_REFERENCE {
                Ok(StartupTarget::HostCodex)
            } else {
                Err(ConfigError::InvalidModel {
                    path: path.to_owned(),
                    message: format!(
                        "model.startup host target must be exactly {}",
                        StartupTarget::HOST_CODEX_REFERENCE
                    ),
                })
            };
        },
        StartupTargetConfig::Model {
            provider,
            account,
            model,
        } => (provider, account, model),
    };
    let startup = ModelSelection::new(
        ProviderId::new(provider).map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        })?,
        AccountId::new(account).map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        })?,
        ModelId::new(model).map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        })?,
    );
    catalog
        .resolve_model(startup.provider(), startup.account(), startup.model())
        .map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: format!("model.startup does not name one configured entry: {error}"),
        })?;
    Ok(StartupTarget::Model(startup))
}

fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = env::var_os("YO_CONFIG") {
        if path.is_empty() {
            return Err(ConfigError::Environment("YO_CONFIG must not be empty"));
        }
        return Ok(PathBuf::from(path));
    }
    default_config_path()
}

#[cfg(target_os = "macos")]
fn default_config_path() -> Result<PathBuf, ConfigError> {
    let home = env::var_os("HOME").ok_or(ConfigError::Environment(
        "HOME is required to locate Yo configuration",
    ))?;
    Ok(environment_root("HOME", home)?
        .join("Library")
        .join("Application Support")
        .join("yo")
        .join("config.yaml"))
}

#[cfg(not(target_os = "macos"))]
fn default_config_path() -> Result<PathBuf, ConfigError> {
    let root = match env::var_os("XDG_CONFIG_HOME") {
        Some(root) if !root.is_empty() => environment_root("XDG_CONFIG_HOME", root)?,
        _ => {
            let home = env::var_os("HOME").ok_or(ConfigError::Environment(
                "HOME is required when XDG_CONFIG_HOME is not set",
            ))?;
            environment_root("HOME", home)?.join(".config")
        },
    };
    Ok(root.join("yo").join("config.yaml"))
}

fn environment_root(name: &'static str, value: OsString) -> Result<PathBuf, ConfigError> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(ConfigError::Environment(match name {
            "HOME" => "HOME must be a non-empty absolute path",
            "XDG_CONFIG_HOME" => "XDG_CONFIG_HOME must be a non-empty absolute path",
            _ => "configuration root must be a non-empty absolute path",
        }));
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
