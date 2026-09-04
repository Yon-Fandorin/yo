use std::{
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
use yo_core::ModelCatalog;

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
    // Runtime model state is injected from one ConnectionRepository snapshot. It is never
    // decoded from config.yaml.
    model_catalog: ModelCatalog,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            date_format: DEFAULT_DATE_FORMAT.to_owned(),
            frame_rate_limit: yo_tui::FrameRateLimit::Fps120,
            source_path: PathBuf::new(),
            snapshot: ConfigSnapshot::absent(),
            model_catalog: ModelCatalog::default(),
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

    pub(crate) fn credential_path(&self) -> PathBuf {
        self.state_directory().join("credentials.yaml")
    }

    pub(crate) fn connection_path(&self) -> PathBuf {
        self.state_directory().join("connections.yaml")
    }

    pub(crate) fn account_capacity_path(&self) -> PathBuf {
        self.state_directory().join("account-capacity.yaml")
    }

    pub(crate) fn state_directory(&self) -> PathBuf {
        self.source_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_owned()
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
}

impl ConfigSnapshot {
    fn absent() -> Self {
        Self {
            metadata: None,
            bytes: Vec::new(),
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
        source: Box<yo_yaml::Error>,
    },
    InvalidDateFormat(String),
    InvalidMaxFps {
        path: PathBuf,
        value: u16,
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
            Self::InvalidYaml { path, source } => write!(
                formatter,
                "{} contains invalid configuration: {source}",
                path.display()
            ),
            Self::InvalidDateFormat(message) => formatter.write_str(message),
            Self::InvalidMaxFps { path, value } => write!(
                formatter,
                "{}: tui.max_fps must be 60 or 120, not {value}",
                path.display()
            ),
            Self::TimestampOutOfRange(millis) => write!(
                formatter,
                "timestamp {millis}ms is outside the supported date range"
            ),
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
            Self::InvalidYaml { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    session: SessionConfig,
    #[serde(default)]
    tui: TuiConfig,
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

pub(crate) fn load() -> Result<Config, ConfigError> {
    load_from(&config_path()?)
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
        },
    )
}

fn parse_snapshot(
    path: &Path,
    contents: &str,
    snapshot: ConfigSnapshot,
) -> Result<Config, ConfigError> {
    let decoded: FileConfig = yo_yaml::from_str_with_limits(
        contents,
        yo_yaml::ParseLimits::with_max_total_scalar_bytes(MAX_CONFIG_BYTES as usize),
    )
    .map_err(|source| ConfigError::InvalidYaml {
        path: path.to_owned(),
        source: Box::new(source),
    })?;
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
    let config = Config {
        date_format: decoded
            .session
            .list
            .date_format
            .unwrap_or_else(|| DEFAULT_DATE_FORMAT.to_owned()),
        frame_rate_limit,
        source_path: path.to_owned(),
        snapshot,
        model_catalog: ModelCatalog::default(),
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
    Ok(ConfigSnapshot {
        metadata: Some(before),
        bytes,
    })
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
