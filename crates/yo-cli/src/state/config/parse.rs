use std::path::Path;

use serde::Deserialize;

use super::{Config, ConfigError, ConfigSnapshot};

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

pub(super) fn parse_snapshot(
    path: &Path,
    contents: &str,
    snapshot: ConfigSnapshot,
) -> Result<Config, ConfigError> {
    let decoded: FileConfig = yo_yaml::from_str_with_limits(
        contents,
        yo_yaml::ParseLimits::with_max_total_scalar_bytes(
            super::snapshot::MAX_CONFIG_BYTES as usize,
        ),
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
            .unwrap_or_else(|| super::date::DEFAULT_DATE_FORMAT.to_owned()),
        frame_rate_limit,
        source_path: path.to_owned(),
        snapshot,
        model_catalog: yo_core::ModelCatalog::default(),
    };
    config.date_formatter().map_err(|error| match error {
        ConfigError::InvalidDateFormat(message) => {
            ConfigError::InvalidDateFormat(format!("{}: {message}", path.display()))
        },
        other => other,
    })?;
    Ok(config)
}
