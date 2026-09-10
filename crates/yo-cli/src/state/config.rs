use std::path::{Path, PathBuf};

use yo_core::{ModelCatalog, SkillReferenceScope};
use yo_tui::{FrameRateLimit, OutputPreferences, PromptTemplates, Theme, ThemeOverrides};

mod commands;
mod date;
mod error;
mod parse;
mod path;
mod snapshot;

pub(crate) use commands::CommandToolConfig;
pub(crate) use date::DateFormatter;
pub(crate) use error::ConfigError;
use parse::parse_snapshot;
use path::config_path;
use snapshot::{ConfigSnapshot, capture_snapshot};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct SkillRootConfig {
    pub(crate) path: PathBuf,
    pub(crate) scope: SkillReferenceScope,
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    skill_roots: Vec<SkillRootConfig>,
    command_tools: Vec<CommandToolConfig>,
    prompts: PromptTemplates,
    date_format: String,
    frame_rate_limit: FrameRateLimit,
    theme: Theme,
    theme_overrides: ThemeOverrides,
    output_preferences: OutputPreferences,
    source_path: PathBuf,
    snapshot: ConfigSnapshot,
    // Runtime model state is injected from one ConnectionRepository snapshot. It is never
    // decoded from config.yaml.
    model_catalog: ModelCatalog,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            skill_roots: Vec::new(),
            command_tools: Vec::new(),
            prompts: PromptTemplates::default(),
            date_format: date::DEFAULT_DATE_FORMAT.to_owned(),
            frame_rate_limit: FrameRateLimit::Fps120,
            theme: Theme::Default,
            theme_overrides: ThemeOverrides::default(),
            output_preferences: OutputPreferences::default(),
            source_path: PathBuf::new(),
            snapshot: ConfigSnapshot::absent(),
            model_catalog: ModelCatalog::default(),
        }
    }
}

impl Config {
    /// Structurally admitted commands, without artifact resolution or file reads.
    pub(crate) fn command_tools(&self) -> &[CommandToolConfig] {
        &self.command_tools
    }

    pub(crate) fn skill_roots(&self) -> &[SkillRootConfig] {
        &self.skill_roots
    }

    pub(crate) fn prompts(&self) -> &PromptTemplates {
        &self.prompts
    }

    pub(crate) fn theme_overrides(&self) -> &ThemeOverrides {
        &self.theme_overrides
    }
    pub(crate) fn output_preferences(&self) -> OutputPreferences {
        self.output_preferences
    }
    pub(crate) fn date_formatter(&self) -> Result<DateFormatter, ConfigError> {
        DateFormatter::new(&self.date_format)
    }

    pub(crate) fn frame_rate_limit(&self) -> FrameRateLimit {
        self.frame_rate_limit
    }

    pub(crate) fn theme(&self) -> Theme {
        self.theme
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
