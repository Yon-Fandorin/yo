use std::{error::Error, fmt, path::PathBuf};

#[derive(Debug)]
pub(crate) enum ConfigError {
    Environment(&'static str),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    UnsupportedFileType(PathBuf),
    TooLarge {
        path: PathBuf,
        limit: u64,
    },
    InvalidUtf8 {
        path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    Changed(PathBuf),
    InvalidYaml {
        path: PathBuf,
        source: Box<yo_yaml::Error>,
    },
    InvalidSkills {
        path: PathBuf,
        detail: String,
    },
    InvalidTools {
        path: PathBuf,
        detail: &'static str,
    },
    InvalidPrompts {
        path: PathBuf,
        detail: String,
    },
    InvalidDateFormat(String),
    InvalidMaxFps {
        path: PathBuf,
        value: u16,
    },
    InvalidThemeColor {
        path: PathBuf,
        role: String,
        value: String,
        detail: &'static str,
    },
    InvalidTheme {
        path: PathBuf,
        value: String,
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
            Self::TooLarge { path, limit } => write!(
                formatter,
                "{} exceeds the {limit}-byte Yo configuration limit",
                path.display(),
            ),
            Self::InvalidUtf8 { path, .. } => {
                write!(formatter, "{} is not valid UTF-8", path.display())
            },
            Self::InvalidYaml { path, source } => write!(
                formatter,
                "{} contains invalid configuration: {source}",
                path.display()
            ),
            Self::InvalidSkills { path, detail } => {
                write!(formatter, "{}: skills: {detail}", path.display())
            },
            Self::InvalidTools { path, detail } => {
                write!(formatter, "{}: tools.commands: {detail}", path.display())
            },
            Self::InvalidPrompts { path, detail } => {
                write!(formatter, "{}: prompts: {detail}", path.display())
            },
            Self::InvalidDateFormat(message) => formatter.write_str(message),
            Self::InvalidMaxFps { path, value } => write!(
                formatter,
                "{}: tui.max_fps must be 60 or 120, not {value}",
                path.display()
            ),
            Self::InvalidThemeColor {
                path,
                role,
                value,
                detail,
            } => write!(
                formatter,
                "{}: tui.colors.{role} ({value:?}): {detail}",
                path.display()
            ),
            Self::InvalidTheme { path, value } => write!(
                formatter,
                "{}: tui.theme must be default, light, or mono, not {value:?}",
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
