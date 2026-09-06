use std::path::{Path, PathBuf};

use yo_core::ModelCatalog;

mod date;
mod error;
mod parse;
mod path;
mod snapshot;

pub(crate) use date::DateFormatter;
pub(crate) use error::ConfigError;
use parse::parse_snapshot;
use path::config_path;
use snapshot::{ConfigSnapshot, capture_snapshot};

#[cfg(test)]
mod tests;

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
            date_format: date::DEFAULT_DATE_FORMAT.to_owned(),
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

pub(crate) fn load() -> Result<Config, ConfigError> {
    load_from(&config_path()?)
}

pub(crate) fn selected_path() -> Result<PathBuf, ConfigError> {
    config_path()
}

pub(crate) fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let snapshot = capture_snapshot(path)?;
    if snapshot.is_absent() {
        return Ok(Config {
            source_path: path.to_owned(),
            snapshot,
            ..Config::default()
        });
    }
    let contents = String::from_utf8(snapshot.bytes().to_owned()).map_err(|source| {
        ConfigError::InvalidUtf8 {
            path: path.to_owned(),
            source,
        }
    })?;
    parse_snapshot(path, &contents, snapshot)
}

#[cfg(test)]
fn parse(path: &Path, contents: &str) -> Result<Config, ConfigError> {
    parse_snapshot(
        path,
        contents,
        ConfigSnapshot::from_bytes(contents.as_bytes().to_vec()),
    )
}
