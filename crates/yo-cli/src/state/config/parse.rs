use std::{collections::BTreeMap, num::NonZeroU16, path::Path};

use serde::{Deserialize, de::Error as DeserializeError};
use yo_core::SkillReferenceScope;
use yo_tui::{OutputPreferences, PromptTemplates, Theme, ThemeOverrides};
use yo_yaml::Error as YamlError;

use super::{
    Config, ConfigError, ConfigSnapshot, SkillRootConfig,
    clipboard::{ClipboardConfig, deserialize_clipboard},
    commands::{ToolsConfig, deserialize_tools, is_tools_decode_error},
    snapshot::MAX_CONFIG_BYTES,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default, deserialize_with = "deserialize_clipboard")]
    clipboard: Option<ClipboardConfig>,
    #[serde(default, deserialize_with = "deserialize_tools")]
    tools: ToolsConfig,
    #[serde(default)]
    skills: SkillsConfig,
    #[serde(default)]
    prompts: BTreeMap<String, String>,
    #[serde(default)]
    session: SessionConfig,
    #[serde(default)]
    tui: TuiConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillsConfig {
    #[serde(default)]
    roots: Vec<SkillRoot>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillRoot {
    path: String,
    scope: SkillScope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SkillScope {
    Workspace,
    User,
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
    theme: Option<String>,
    #[serde(default)]
    colors: BTreeMap<String, String>,
    max_body_width: Option<NonZeroU16>,
    tool_head_rows: Option<u16>,
    shell_tail_rows: Option<u16>,
    diff_head_rows: Option<u16>,
    show_images: Option<bool>,
    hyperlinks: Option<bool>,
    show_reasoning: Option<bool>,
    show_diagrams: Option<bool>,
    image_max_width: Option<NonZeroU16>,
    code_padding: Option<u16>,
}

pub(super) fn parse_snapshot(
    path: &Path,
    contents: &str,
    snapshot: ConfigSnapshot,
) -> Result<Config, ConfigError> {
    let decoded: FileConfig = yo_yaml::from_str_with_limits(
        contents,
        yo_yaml::ParseLimits::with_max_total_scalar_bytes(
            MAX_CONFIG_BYTES as usize,
        ),
    )
    .map_err(|source| {
        if is_tools_decode_error(&source) {
            ConfigError::InvalidTools {
                path: path.to_owned(),
                detail: "invalid structure; check required fields, value types, duplicates and unknown fields",
            }
        } else {
            // A parser snippet can include tools even when the failure occurs in
            // another root field. Keep the original diagnostic only after a
            // bounded structural parse proves that this document has no tools.
            let source = match yo_yaml::has_any_top_level_mapping_key(contents.as_bytes(), &["tools"]) {
                Ok(false) => source,
                Ok(true) | Err(_) => YamlError::custom(
                    "invalid configuration structure; check required fields, value types, duplicates and YAML syntax",
                ),
            };
            ConfigError::InvalidYaml {
                path: path.to_owned(),
                source: Box::new(source),
            }
        }
    })?;
    if decoded.skills.roots.len() > 16 {
        return Err(ConfigError::InvalidSkills {
            path: path.to_owned(),
            detail: "skills.roots supports at most 16 explicitly configured roots".to_owned(),
        });
    }
    let skill_roots = decoded
        .skills
        .roots
        .into_iter()
        .map(|root| {
            if root.path.is_empty()
                || root.path.len() > 4096
                || root.path.chars().any(char::is_control)
            {
                return Err(ConfigError::InvalidSkills {
                    path: path.to_owned(),
                    detail:
                        "each root path must contain 1 to 4096 bytes without control characters"
                            .to_owned(),
                });
            }
            Ok(SkillRootConfig {
                path: root.path.into(),
                scope: match root.scope {
                    SkillScope::Workspace => SkillReferenceScope::Workspace,
                    SkillScope::User => SkillReferenceScope::User,
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let prompts =
        PromptTemplates::new(decoded.prompts).map_err(|error| ConfigError::InvalidPrompts {
            path: path.to_owned(),
            detail: error.to_string(),
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
    let theme = decoded
        .tui
        .theme
        .map(|value| {
            value
                .parse::<Theme>()
                .map_err(|_| ConfigError::InvalidTheme {
                    path: path.to_owned(),
                    value,
                })
        })
        .transpose()?
        .unwrap_or_default();
    let mut theme_overrides = ThemeOverrides::default();
    for (role, value) in decoded.tui.colors {
        let parsed_role = role
            .parse()
            .map_err(|detail| ConfigError::InvalidThemeColor {
                path: path.to_owned(),
                role: role.clone(),
                value: value.clone(),
                detail,
            })?;
        let color = value
            .parse()
            .map_err(|detail| ConfigError::InvalidThemeColor {
                path: path.to_owned(),
                role: role.clone(),
                value: value.clone(),
                detail,
            })?;
        theme_overrides = theme_overrides.with_color(parsed_role, color);
    }
    let config = Config {
        skill_roots,
        command_tools: decoded.tools.into_commands(path)?,
        clipboard_source: decoded
            .clipboard
            .map(|source| source.admit(path))
            .transpose()?,
        prompts,
        theme_overrides,
        output_preferences: OutputPreferences::default()
            .with_max_body_width(decoded.tui.max_body_width)
            .with_code_padding(decoded.tui.code_padding.unwrap_or(1))
            .with_tool_head_rows(decoded.tui.tool_head_rows.unwrap_or(2))
            .with_shell_tail_rows(decoded.tui.shell_tail_rows.unwrap_or(5))
            .with_diff_head_rows(decoded.tui.diff_head_rows.unwrap_or(6))
            .with_images(decoded.tui.show_images.unwrap_or(true))
            .with_hyperlinks(decoded.tui.hyperlinks.unwrap_or(true))
            .with_reasoning(decoded.tui.show_reasoning.unwrap_or(true))
            .with_diagrams(decoded.tui.show_diagrams.unwrap_or(true))
            .with_image_max_width(
                decoded
                    .tui
                    .image_max_width
                    .unwrap_or(NonZeroU16::new(64).unwrap()),
            ),
        date_format: decoded
            .session
            .list
            .date_format
            .unwrap_or_else(|| super::date::DEFAULT_DATE_FORMAT.to_owned()),
        frame_rate_limit,
        theme,
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
