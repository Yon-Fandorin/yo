use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt, fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use jiff::{Timestamp, fmt::strtime, tz::TimeZone};
use serde::Deserialize;
use yo_core::{
    AccountId, EffectiveModelBinding, ModelCatalog, ModelCatalogEntry, ModelContextProfile,
    ModelId, NormalizedEndpoint, ProviderId,
};

const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d %H:%M %:z";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_DATE_FORMAT_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct Config {
    date_format: String,
    frame_rate_limit: yo_tui::FrameRateLimit,
    source_path: PathBuf,
    model_catalog: ModelCatalog,
    startup_model: Option<StartupModel>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            date_format: DEFAULT_DATE_FORMAT.to_owned(),
            frame_rate_limit: yo_tui::FrameRateLimit::Fps120,
            source_path: PathBuf::new(),
            model_catalog: ModelCatalog::default(),
            startup_model: None,
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

    pub(crate) fn startup_model(&self) -> Option<&StartupModel> {
        self.startup_model.as_ref()
    }

    pub(crate) fn credential_path(&self) -> PathBuf {
        self.source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("credentials.yaml")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StartupModel {
    provider: ProviderId,
    account: AccountId,
    model: ModelId,
}

impl StartupModel {
    pub(crate) fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub(crate) fn account(&self) -> &AccountId {
        &self.account
    }

    pub(crate) fn model(&self) -> &ModelId {
        &self.model
    }
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
    startup: Option<StartupModelConfig>,
    #[serde(default)]
    catalog: Vec<ModelEntryConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupModelConfig {
    provider: String,
    account: String,
    model: String,
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

pub(crate) fn load() -> Result<Config, ConfigError> {
    let path = config_path()?;
    load_from(&path)
}

fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Config {
                source_path: path.to_owned(),
                ..Config::default()
            });
        },
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_owned(),
                source,
            });
        },
    };
    let metadata = file.metadata().map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ConfigError::UnsupportedFileType(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len().min(MAX_CONFIG_BYTES)).unwrap_or(MAX_CONFIG_BYTES as usize),
    );
    file.by_ref()
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge(path.to_owned()));
    }
    let contents = String::from_utf8(bytes).map_err(|source| ConfigError::InvalidUtf8 {
        path: path.to_owned(),
        source,
    })?;
    parse(path, &contents)
}

fn parse(path: &Path, contents: &str) -> Result<Config, ConfigError> {
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
    let model_catalog = ModelCatalog::new(
        decoded
            .model
            .catalog
            .into_iter()
            .map(|entry| model_entry(path, entry))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|error| ConfigError::InvalidModel {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let startup_model = decoded
        .model
        .startup
        .map(|startup| startup_model(path, startup, &model_catalog))
        .transpose()?;
    let config = Config {
        date_format: decoded
            .session
            .list
            .date_format
            .unwrap_or_else(|| DEFAULT_DATE_FORMAT.to_owned()),
        frame_rate_limit,
        source_path: path.to_owned(),
        model_catalog,
        startup_model,
    };
    config.date_formatter().map_err(|error| match error {
        ConfigError::InvalidDateFormat(message) => {
            ConfigError::InvalidDateFormat(format!("{}: {message}", path.display()))
        },
        other => other,
    })?;
    Ok(config)
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

fn startup_model(
    path: &Path,
    startup: StartupModelConfig,
    catalog: &ModelCatalog,
) -> Result<StartupModel, ConfigError> {
    let startup = StartupModel {
        provider: ProviderId::new(startup.provider).map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        })?,
        account: AccountId::new(startup.account).map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        })?,
        model: ModelId::new(startup.model).map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: error.to_string(),
        })?,
    };
    catalog
        .resolve_model(&startup.provider, &startup.account, &startup.model)
        .map_err(|error| ConfigError::InvalidModel {
            path: path.to_owned(),
            message: format!("model.startup does not name one configured entry: {error}"),
        })?;
    Ok(startup)
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
