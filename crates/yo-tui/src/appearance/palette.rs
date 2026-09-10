//! Semantic palette resolution. Components receive complete styles, never theme names.

use std::{collections::BTreeMap, str::FromStr};

use super::{ActivityStyles, ColorCapability, GlyphProfile};
use crate::{
    overlay::{SelectionPanelAppearance, SelectionPanelGlyphs, SelectionPanelStyles},
    prompt::{PromptGlyphs, PromptStyles},
    shell::{AgentShellStyles, ShellChromeStyles},
    surface::{Attributes, Color, Style},
    transcript::{MarkdownStyles, TranscriptActivityStyles, TranscriptStyles},
};

/// Built-in palettes, independent of terminal glyphs and motion preferences.
/// The default and light palettes inherit the terminal's body colors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Theme {
    /// Teal focus and a slate request band, matching the original chat appearance.
    #[default]
    Default,
    /// Deep teal focus and a pale request band for light terminal backgrounds.
    Light,
    /// Terminal colors only, with emphasis conveyed through attributes and spacing.
    Mono,
}

impl FromStr for Theme {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "default" => Ok(Self::Default),
            "light" => Ok(Self::Light),
            "mono" => Ok(Self::Mono),
            _ => Err("expected default, light, or mono"),
        }
    }
}

/// Semantic color slots shared by output components.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[non_exhaustive]
pub enum ThemeRole {
    /// Focus, headings, links and selection.
    Accent,
    /// Chart traces, bars and axes; inherits accent unless overridden.
    Chart,
    /// Second named chart series.
    Chart2,
    /// Third named chart series.
    Chart3,
    /// Fourth named chart series.
    Chart4,
    /// Assistant text and prompt text.
    Text,
    /// Tool body, quotes and secondary hints.
    Muted,
    /// Public reasoning summary text and its hidden-state hint.
    ReasoningText,
    /// User message text.
    UserText,
    /// User message band.
    UserBackground,
    /// Unclassified code text and code labels.
    CodeText,
    /// Code body and syntax token backgrounds.
    CodeBackground,
    /// Code language header background.
    CodeHeaderBackground,
    /// Added lines.
    DiffAdded,
    /// Removed lines.
    DiffRemoved,
    /// Added line background.
    DiffAddedBackground,
    /// Removed line background.
    DiffRemovedBackground,
    /// Successful activity outcome.
    Success,
    /// Interrupted activity and warning status.
    Warning,
    /// Failed activity outcome.
    Error,
    /// Language keywords.
    SyntaxKeyword,
    /// String literals.
    SyntaxString,
    /// Comments.
    SyntaxComment,
    /// Numeric literals.
    SyntaxNumber,
    /// Function names.
    SyntaxFunction,
    /// Type names.
    SyntaxType,
}

impl FromStr for ThemeRole {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "accent" => Ok(Self::Accent),
            "chart" => Ok(Self::Chart),
            "chart_2" => Ok(Self::Chart2),
            "chart_3" => Ok(Self::Chart3),
            "chart_4" => Ok(Self::Chart4),
            "text" => Ok(Self::Text),
            "muted" => Ok(Self::Muted),
            "reasoning_text" => Ok(Self::ReasoningText),
            "user_text" => Ok(Self::UserText),
            "user_background" => Ok(Self::UserBackground),
            "code_text" => Ok(Self::CodeText),
            "code_background" => Ok(Self::CodeBackground),
            "code_header_background" => Ok(Self::CodeHeaderBackground),
            "diff_added" => Ok(Self::DiffAdded),
            "diff_removed" => Ok(Self::DiffRemoved),
            "diff_added_background" => Ok(Self::DiffAddedBackground),
            "diff_removed_background" => Ok(Self::DiffRemovedBackground),
            "success" => Ok(Self::Success),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            "syntax_keyword" => Ok(Self::SyntaxKeyword),
            "syntax_string" => Ok(Self::SyntaxString),
            "syntax_comment" => Ok(Self::SyntaxComment),
            "syntax_number" => Ok(Self::SyntaxNumber),
            "syntax_function" => Ok(Self::SyntaxFunction),
            "syntax_type" => Ok(Self::SyntaxType),
            _ => Err("unknown semantic theme color role"),
        }
    }
}

/// A terminal default color or an RGB value with an automatic 256-color fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeColor {
    /// Inherit the terminal color.
    Terminal,
    /// Explicit red, green and blue channels.
    Rgb(u8, u8, u8),
}

impl FromStr for ThemeColor {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "terminal" {
            return Ok(Self::Terminal);
        }
        if value.len() != 7 || !value.starts_with('#') || !value.is_ascii() {
            return Err("expected #RRGGBB or terminal");
        }
        let channel = |offset| {
            u8::from_str_radix(&value[offset..offset + 2], 16)
                .map_err(|_| "expected #RRGGBB or terminal")
        };
        Ok(Self::Rgb(channel(1)?, channel(3)?, channel(5)?))
    }
}

impl ThemeColor {
    fn resolve(self, capability: ColorCapability) -> Color {
        let Self::Rgb(red, green, blue) = self else {
            return Color::Default;
        };
        match capability {
            ColorCapability::TrueColor => Color::Rgb { red, green, blue },
            ColorCapability::Unknown => Color::Default,
            ColorCapability::Limited => {
                let levels = [0_i32, 95, 135, 175, 215, 255];
                let index = (16_u16..=255)
                    .min_by_key(|index| {
                        let channels = if *index >= 232 {
                            let gray = 8 + (i32::from(*index) - 232) * 10;
                            [gray; 3]
                        } else {
                            let cube = usize::from(*index - 16);
                            [levels[cube / 36], levels[cube / 6 % 6], levels[cube % 6]]
                        };
                        [red, green, blue]
                            .into_iter()
                            .zip(channels)
                            .map(|(value, target)| (i32::from(value) - target).pow(2))
                            .sum::<i32>()
                    })
                    .expect("the indexed palette is nonempty");
                Color::Indexed(index as u8)
            },
        }
    }
}

/// User color overrides layered over a built-in theme. Unset roles retain defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThemeOverrides(BTreeMap<ThemeRole, ThemeColor>);

impl ThemeOverrides {
    /// Sets one semantic color without changing emphasis, glyphs or layout.
    #[must_use]
    pub fn with_color(mut self, role: ThemeRole, color: ThemeColor) -> Self {
        self.0.insert(role, color);
        self
    }

    pub(super) fn apply(
        &self,
        styles: &mut AgentShellStyles,
        capability: ColorCapability,
        theme: Theme,
    ) {
        let capability = if theme == Theme::Mono {
            ColorCapability::Unknown
        } else {
            capability
        };
        for (&role, &value) in &self.0 {
            let color = value.resolve(capability);
            let transcript = &mut styles.transcript;
            let markdown = &mut transcript.markdown;
            match role {
                ThemeRole::Accent => {
                    transcript.assistant_marker.foreground = color;
                    transcript.activity.heading.foreground = color;
                    markdown.heading.foreground = color;
                    markdown.chart[0].foreground = color;
                    markdown.link.foreground = color;
                    markdown.diff_meta.foreground = color;
                    styles.prompt.marker.foreground = color;
                    styles.chrome.key_hint.foreground = color;
                    styles.overlay.styles.selected.foreground = color;
                },
                ThemeRole::Chart => markdown.chart[0].foreground = color,
                ThemeRole::Chart2 => markdown.chart[1].foreground = color,
                ThemeRole::Chart3 => markdown.chart[2].foreground = color,
                ThemeRole::Chart4 => markdown.chart[3].foreground = color,
                ThemeRole::Text => {
                    transcript.background.foreground = color;
                    transcript.assistant_body.foreground = color;
                    styles.prompt.body.foreground = color;
                    styles.overlay.styles.label.foreground = color;
                },
                ThemeRole::Muted => {
                    transcript.activity.body.foreground = color;
                    transcript.activity.reasoning.foreground = color;
                    markdown.quote.foreground = color;
                    styles.prompt.rule.foreground = color;
                    styles.chrome.metrics.foreground = color;
                    styles.chrome.mode.foreground = color;
                    styles.overlay.styles.hint.foreground = color;
                    styles.overlay.styles.detail.foreground = color;
                },
                ThemeRole::ReasoningText => transcript.activity.reasoning.foreground = color,
                ThemeRole::UserText => {
                    transcript.user_body.foreground = color;
                    transcript.user_marker.foreground = color;
                },
                ThemeRole::UserBackground => {
                    transcript.user_body.background = color;
                    transcript.user_marker.background = color;
                },
                ThemeRole::CodeText => {
                    markdown.code.foreground = color;
                    markdown.code_label.foreground = color;
                },
                ThemeRole::CodeBackground => {
                    markdown.code.background = color;
                    markdown.diff_meta.background = color;
                    for syntax in &mut markdown.syntax {
                        syntax.background = color;
                    }
                },
                ThemeRole::CodeHeaderBackground => markdown.code_label.background = color,
                ThemeRole::DiffAdded => markdown.diff_added.foreground = color,
                ThemeRole::DiffRemoved => markdown.diff_removed.foreground = color,
                ThemeRole::DiffAddedBackground => markdown.diff_added.background = color,
                ThemeRole::DiffRemovedBackground => markdown.diff_removed.background = color,
                ThemeRole::Success => transcript.activity.success.foreground = color,
                ThemeRole::Warning => transcript.activity.warning.foreground = color,
                ThemeRole::Error => transcript.activity.error.foreground = color,
                ThemeRole::SyntaxKeyword => markdown.syntax[0].foreground = color,
                ThemeRole::SyntaxString => markdown.syntax[1].foreground = color,
                ThemeRole::SyntaxComment => markdown.syntax[2].foreground = color,
                ThemeRole::SyntaxNumber => markdown.syntax[3].foreground = color,
                ThemeRole::SyntaxFunction => markdown.syntax[4].foreground = color,
                ThemeRole::SyntaxType => markdown.syntax[5].foreground = color,
            }
        }
    }
}

// Every explicit color has an indexed fallback; Unknown always inherits the host.
#[derive(Clone, Copy)]
struct PaletteColor {
    rgb: (u8, u8, u8),
    indexed: u8,
}

impl PaletteColor {
    const fn new(rgb: (u8, u8, u8), indexed: u8) -> Self {
        Self { rgb, indexed }
    }

    const fn resolve(self, capability: ColorCapability) -> Color {
        match capability {
            ColorCapability::TrueColor => Color::Rgb {
                red: self.rgb.0,
                green: self.rgb.1,
                blue: self.rgb.2,
            },
            ColorCapability::Limited => Color::Indexed(self.indexed),
            ColorCapability::Unknown => Color::Default,
        }
    }
}

struct Palette {
    focus: PaletteColor,
    request_marker: PaletteColor,
    request_foreground: PaletteColor,
    request_background: PaletteColor,
    code_background: PaletteColor,
    code_header: PaletteColor,
    added_background: PaletteColor,
    removed_background: PaletteColor,
    success: PaletteColor,
    warning: PaletteColor,
    error: PaletteColor,
}

impl Theme {
    const fn palette(self) -> Palette {
        match self {
            Self::Default | Self::Mono => Palette {
                focus: PaletteColor::new((94, 179, 179), 73),
                request_marker: PaletteColor::new((124, 151, 199), 110),
                request_foreground: PaletteColor::new((222, 230, 242), 254),
                request_background: PaletteColor::new((32, 41, 54), 236),
                code_background: PaletteColor::new((30, 38, 49), 235),
                code_header: PaletteColor::new((43, 56, 70), 238),
                added_background: PaletteColor::new((27, 55, 43), 22),
                removed_background: PaletteColor::new((62, 34, 42), 52),
                success: PaletteColor::new((138, 190, 143), 114),
                warning: PaletteColor::new((218, 183, 117), 179),
                error: PaletteColor::new((223, 130, 139), 174),
            },
            Self::Light => Palette {
                focus: PaletteColor::new((25, 105, 109), 23),
                request_marker: PaletteColor::new((53, 81, 130), 24),
                request_foreground: PaletteColor::new((36, 49, 66), 236),
                request_background: PaletteColor::new((231, 237, 245), 255),
                code_background: PaletteColor::new((241, 244, 248), 255),
                code_header: PaletteColor::new((221, 230, 240), 253),
                added_background: PaletteColor::new((220, 242, 226), 194),
                removed_background: PaletteColor::new((250, 225, 229), 224),
                success: PaletteColor::new((42, 112, 65), 29),
                warning: PaletteColor::new((135, 87, 20), 94),
                error: PaletteColor::new((166, 50, 65), 124),
            },
        }
    }

    pub(super) const fn activity_colors(self) -> ((u8, u8, u8), (u8, u8, u8)) {
        match self {
            Self::Default | Self::Mono => ((128, 128, 128), (255, 255, 255)),
            Self::Light => ((112, 122, 132), (28, 46, 64)),
        }
    }
}

pub(super) fn styles(
    profile: GlyphProfile,
    capability: ColorCapability,
    theme: Theme,
) -> AgentShellStyles {
    let mut styles = resolve_styles(
        match profile {
            GlyphProfile::Rich => PromptGlyphs::rich(),
            GlyphProfile::Ascii => PromptGlyphs::ascii(),
        },
        match profile {
            GlyphProfile::Rich => SelectionPanelGlyphs::rich(),
            GlyphProfile::Ascii => SelectionPanelGlyphs::ascii(),
        },
        capability,
        theme,
    );
    styles.transcript.markdown.rich_media = profile == GlyphProfile::Rich;
    styles
}

pub(super) fn apply(styles: &mut AgentShellStyles, capability: ColorCapability, theme: Theme) {
    let rich_media = styles.transcript.markdown.rich_media;
    *styles = resolve_styles(
        styles.prompt.glyphs,
        styles.overlay.glyphs,
        capability,
        theme,
    );
    styles.transcript.markdown.rich_media = rich_media;
}

fn resolve_styles(
    prompt_glyphs: PromptGlyphs,
    overlay_glyphs: SelectionPanelGlyphs,
    capability: ColorCapability,
    theme: Theme,
) -> AgentShellStyles {
    let capability = if theme == Theme::Mono {
        ColorCapability::Unknown
    } else {
        capability
    };
    let palette = theme.palette();
    let neutral = Style::default();
    let muted = Style::new(Color::Default, Color::Default, Attributes::DIM);
    let strong = Style::new(Color::Default, Color::Default, Attributes::BOLD);
    let focus = palette.focus.resolve(capability);
    let accent = Style::new(focus, Color::Default, Attributes::empty());
    let accent_strong = Style::new(focus, Color::Default, Attributes::BOLD);
    let request_background = palette.request_background.resolve(capability);
    AgentShellStyles {
        transcript: TranscriptStyles {
            background: neutral,
            user_marker: Style::new(
                palette.request_marker.resolve(capability),
                request_background,
                Attributes::BOLD,
            ),
            user_body: Style::new(
                palette.request_foreground.resolve(capability),
                request_background,
                Attributes::BOLD,
            ),
            assistant_marker: accent,
            assistant_body: neutral,
            activity: TranscriptActivityStyles {
                heading: accent,
                body: muted,
                reasoning: muted,
                success: Style::new(
                    palette.success.resolve(capability),
                    Color::Default,
                    Attributes::empty(),
                ),
                warning: Style::new(
                    palette.warning.resolve(capability),
                    Color::Default,
                    Attributes::empty(),
                ),
                error: Style::new(
                    palette.error.resolve(capability),
                    Color::Default,
                    Attributes::empty(),
                ),
            },
            markdown: MarkdownStyles {
                pixel_color_capability: palette.request_foreground.resolve(capability),
                heading: accent_strong,
                chart: {
                    let colors = if theme == Theme::Light {
                        [
                            ((153, 67, 123), 132),
                            ((155, 102, 20), 136),
                            ((42, 98, 161), 25),
                        ]
                    } else {
                        [
                            ((212, 145, 193), 175),
                            ((222, 181, 105), 179),
                            ((126, 178, 216), 110),
                        ]
                    };
                    let extra = colors.map(|(rgb, indexed)| {
                        Style::new(
                            PaletteColor::new(rgb, indexed).resolve(capability),
                            Color::Default,
                            Attributes::BOLD,
                        )
                    });
                    [accent_strong, extra[0], extra[1], extra[2]]
                },
                rich_media: true,
                syntax: {
                    let colors = if theme == Theme::Light {
                        [
                            ((123, 54, 136), 96),
                            ((45, 113, 67), 29),
                            ((110, 122, 136), 243),
                            ((160, 82, 24), 130),
                            ((34, 91, 147), 25),
                            ((139, 94, 18), 94),
                        ]
                    } else {
                        [
                            ((184, 148, 204), 140),
                            ((156, 193, 139), 150),
                            ((131, 146, 166), 103),
                            ((218, 161, 119), 180),
                            ((126, 178, 216), 110),
                            ((220, 193, 132), 180),
                        ]
                    };
                    colors.map(|(rgb, indexed)| {
                        Style::new(
                            PaletteColor::new(rgb, indexed).resolve(capability),
                            palette.code_background.resolve(capability),
                            Attributes::empty(),
                        )
                    })
                },
                code: Style::new(
                    palette.request_foreground.resolve(capability),
                    palette.code_background.resolve(capability),
                    Attributes::empty(),
                ),
                code_label: Style::new(
                    palette.request_foreground.resolve(capability),
                    palette.code_header.resolve(capability),
                    Attributes::BOLD,
                ),
                quote: muted,
                link: Style::new(focus, Color::Default, Attributes::UNDERLINE),
                diff_added: Style::new(
                    palette.success.resolve(capability),
                    palette.added_background.resolve(capability),
                    Attributes::empty(),
                ),
                diff_removed: Style::new(
                    palette.error.resolve(capability),
                    palette.removed_background.resolve(capability),
                    Attributes::empty(),
                ),
                diff_meta: Style::new(
                    focus,
                    palette.code_background.resolve(capability),
                    Attributes::empty(),
                ),
            },
        },
        prompt: PromptStyles {
            body: neutral,
            marker: accent_strong,
            rule: muted,
            glyphs: prompt_glyphs,
        },
        chrome: ShellChromeStyles {
            activity: ActivityStyles::built_in(),
            metrics: muted,
            mode: muted,
            key_hint: accent_strong,
        },
        overlay: SelectionPanelAppearance {
            glyphs: overlay_glyphs,
            styles: SelectionPanelStyles {
                activity: ActivityStyles::built_in(),
                background: neutral,
                frame: muted,
                title: strong,
                key_hint: strong,
                hint: muted,
                label: neutral,
                detail: muted,
                selected: accent,
                disabled: muted,
            },
        },
    }
}
