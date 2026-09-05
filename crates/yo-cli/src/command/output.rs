use clap::ValueEnum;
use yo_tui::GlyphProfile;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputOptions {
    pub(crate) format: OutputFormat,
    pub(crate) glyph_profile: GlyphProfile,
}

impl OutputOptions {
    pub(super) fn from_cli(format: OutputFormat, ascii: bool) -> Self {
        Self {
            format,
            glyph_profile: glyph_profile(ascii),
        }
    }
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self::from_cli(OutputFormat::Text, false)
    }
}

fn glyph_profile(ascii: bool) -> GlyphProfile {
    if ascii {
        GlyphProfile::Ascii
    } else {
        GlyphProfile::Rich
    }
}

pub(super) fn validate(
    output: OutputOptions,
    command: &str,
    supports_json: bool,
    supports_ascii: bool,
) -> Result<(), clap::Error> {
    if output.format == OutputFormat::Json && !supports_json {
        return Err(super::raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            format!("--format json is not supported by `{command}`"),
        ));
    }
    if output.glyph_profile == GlyphProfile::Ascii && !supports_ascii {
        return Err(super::raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            format!("--ascii is not supported by `{command}`"),
        ));
    }
    Ok(())
}
