use std::{fmt::Write as _, io::IsTerminal as _, num::NonZeroU16};

use unicode_segmentation::UnicodeSegmentation;
use yo_core::CompleteModelBinding;
use yo_tui::surface::{Grapheme, GraphemeError};

use crate::interaction::{PresentationStyle, TextStyle};

#[cfg(test)]
mod tests;

const DEFAULT_WIDTH: u16 = 80;
const FIELD_LABEL_WIDTH: usize = 16;
const FIELD_INDENT: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanAction {
    Add,
    Change,
    Remove,
    Keep,
    Attention,
    Success,
}

impl PlanAction {
    pub(crate) const fn marker(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Change => "~",
            Self::Remove => "−",
            Self::Keep => "=",
            Self::Attention => "!",
            Self::Success => "✓",
        }
    }

    pub(crate) const fn text_style(self) -> TextStyle {
        match self {
            Self::Add | Self::Success => TextStyle::Positive,
            Self::Change | Self::Attention => TextStyle::Warning,
            Self::Remove => TextStyle::Danger,
            Self::Keep => TextStyle::Muted,
        }
    }
}

#[derive(Default)]
pub(crate) struct PlanCounts {
    add: usize,
    change: usize,
    remove: usize,
    keep: usize,
}

impl PlanCounts {
    pub(crate) fn record(&mut self, action: PlanAction) {
        match action {
            PlanAction::Add => self.add += 1,
            PlanAction::Change => self.change += 1,
            PlanAction::Remove => self.remove += 1,
            PlanAction::Keep => self.keep += 1,
            PlanAction::Attention | PlanAction::Success => {},
        }
    }

    pub(crate) fn sentence(&self) -> String {
        let mut parts = Vec::new();
        for (count, verb) in [
            (self.add, "add"),
            (self.remove, "remove"),
            (self.change, "change"),
            (self.keep, "keep"),
        ] {
            if count > 0 {
                parts.push(format!("{count} to {verb}"));
            }
        }
        format!("Plan: {}.", parts.join(", "))
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct BindingDetails {
    pub(crate) model: String,
    pub(crate) profile: ProfileDetails,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProfileDetails {
    endpoint: String,
    protocol: String,
    connector: String,
    tokenizer: String,
    input_limit: u64,
    output_limit: Option<u64>,
    reasoning: String,
    request_options: String,
    tools: String,
    pub(crate) replay: String,
}

impl BindingDetails {
    pub(crate) fn escape_remote_model(&mut self, model_id: &str) {
        if self.model == model_id {
            self.model = escape_remote_text(&display_model_item(model_id));
        }
    }

    pub(crate) fn render(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        push_detail_field(output, "Model", &self.model, width, style)?;
        self.profile.render(output, width, style)
    }
}

impl ProfileDetails {
    pub(crate) fn render(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        push_detail_field(output, "Endpoint", &self.endpoint, width, style)?;
        push_detail_field(output, "Protocol", &self.protocol, width, style)?;
        push_detail_field(output, "Connector", &self.connector, width, style)?;
        push_detail_field(
            output,
            "Limits",
            &format!(
                "{} input · {} max output tokens",
                readable_number(self.input_limit),
                self.output_limit
                    .map(readable_number)
                    .unwrap_or_else(|| "unknown".to_owned())
            ),
            width,
            style,
        )?;
        push_detail_field(output, "Tokenizer", &self.tokenizer, width, style)?;
        push_detail_field(output, "Tools", &self.tools, width, style)?;
        push_detail_field(output, "Replay", &self.replay, width, style)?;
        push_detail_field(output, "Reasoning", &self.reasoning, width, style)?;
        push_detail_field(
            output,
            "Request options",
            &self.request_options,
            width,
            style,
        )
    }
}

pub(crate) struct ProfileGroup<'a> {
    pub(crate) profile: &'a ProfileDetails,
    pub(crate) models: Vec<&'a str>,
}

pub(crate) fn group_profiles(bindings: &[BindingDetails]) -> Vec<ProfileGroup<'_>> {
    let mut groups: Vec<ProfileGroup<'_>> = Vec::new();
    for binding in bindings {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.profile == &binding.profile)
        {
            group.models.push(&binding.model);
        } else {
            groups.push(ProfileGroup {
                profile: &binding.profile,
                models: vec![&binding.model],
            });
        }
    }
    groups
}

impl From<&CompleteModelBinding> for BindingDetails {
    fn from(complete: &CompleteModelBinding) -> Self {
        let binding = complete.binding();
        let profile = complete.profile();
        Self {
            model: binding.model_id().to_string(),
            profile: ProfileDetails {
                endpoint: binding.endpoint().to_string(),
                protocol: binding.api_dialect().to_string(),
                connector: binding.connector_id().to_string(),
                tokenizer: profile.context().tokenizer_profile().to_owned(),
                input_limit: profile.context().input_token_limit(),
                output_limit: profile.context().max_output_tokens(),
                reasoning: profile.reasoning_parameters().to_json_value().to_string(),
                request_options: profile
                    .optional_request_parameters()
                    .to_json_value()
                    .to_string(),
                tools: profile.tool_capability_policy().to_string(),
                replay: profile.replay_profile().to_string(),
            },
        }
    }
}

pub(crate) trait ConfirmationView {
    fn render_styled(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError>;
    fn prompt(&self) -> &'static str;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PresentationError {
    UnsafeText(GraphemeError),
    GraphemeExceedsWidth { grapheme_width: usize, width: usize },
    InvalidPlan(&'static str),
}

impl std::fmt::Display for PresentationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafeText(error) => write!(
                formatter,
                "connection preview is not terminal-safe: {error}"
            ),
            Self::GraphemeExceedsWidth {
                grapheme_width,
                width,
            } => write!(
                formatter,
                "a {grapheme_width}-cell preview grapheme cannot fit the {width}-cell terminal width"
            ),
            Self::InvalidPlan(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PresentationError {}

impl From<GraphemeError> for PresentationError {
    fn from(error: GraphemeError) -> Self {
        Self::UnsafeText(error)
    }
}

pub(crate) fn default_width() -> NonZeroU16 {
    NonZeroU16::new(DEFAULT_WIDTH).expect("the default terminal width is nonzero")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SuccessPresentation {
    width: NonZeroU16,
    style: PresentationStyle,
}

impl SuccessPresentation {
    pub(crate) fn for_stdout() -> Self {
        let stdout = std::io::stdout();
        let terminal = stdout.is_terminal();
        Self::for_output(&stdout, terminal, std::env::var_os("NO_COLOR").is_some())
    }

    fn for_output(output: &impl std::os::fd::AsFd, terminal: bool, no_color: bool) -> Self {
        let width = terminal
            .then(|| rustix::termios::tcgetwinsize(output).ok())
            .flatten()
            .and_then(|size| NonZeroU16::new(size.ws_col))
            .unwrap_or_else(default_width);
        let style = PresentationStyle::for_output(terminal, no_color);
        Self { width, style }
    }

    #[cfg(test)]
    pub(crate) const fn plain(width: NonZeroU16) -> Self {
        Self {
            width,
            style: PresentationStyle::Plain,
        }
    }

    #[cfg(test)]
    pub(crate) const fn ansi(width: NonZeroU16) -> Self {
        Self {
            width,
            style: PresentationStyle::Ansi,
        }
    }
}

pub(crate) fn render_success(
    presentation: SuccessPresentation,
    heading: &str,
    label_width: usize,
    fields: &[(&str, String)],
) -> Result<String, PresentationError> {
    let width = usize::from(presentation.width.get());
    let style = presentation.style;
    let mut output = String::new();
    push_success_heading(&mut output, heading, width, style)?;
    output.push('\n');
    for (label, value) in fields {
        push_success_field(&mut output, label, value, label_width, width)?;
    }
    Ok(output)
}

fn push_success_heading(
    output: &mut String,
    heading: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline = format!("✓ {heading}");
    if safe_width(&inline)? <= width {
        style.push(output, TextStyle::Positive, "✓");
        output.push(' ');
        style.push(output, TextStyle::Bold, heading);
        output.push('\n');
        return Ok(());
    }
    style.push(output, TextStyle::Positive, "✓");
    output.push('\n');
    for line in wrap(heading, width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

fn push_success_field(
    output: &mut String,
    label: &str,
    value: &str,
    label_width: usize,
    width: usize,
) -> Result<(), PresentationError> {
    let inline_prefix = FIELD_INDENT + label_width;
    let inline_width = width.saturating_sub(inline_prefix);
    if safe_width(label)? <= label_width
        && width > inline_prefix
        && widest_grapheme(value)? <= inline_width
    {
        let prefix = format!("  {label:<label_width$}");
        let continuation = " ".repeat(inline_prefix);
        for (index, line) in wrap(value, inline_width)?.iter().enumerate() {
            output.push_str(if index == 0 { &prefix } else { &continuation });
            output.push_str(line);
            output.push('\n');
        }
    } else {
        let content_width = width;
        for line in wrap(label, content_width)? {
            output.push_str(&line);
            output.push('\n');
        }
        for line in wrap(value, content_width)? {
            output.push_str(&line);
            output.push('\n');
        }
    }
    Ok(())
}

pub(crate) fn push_title(
    output: &mut String,
    title: &str,
    target: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline = format!("{title}  {target}");
    if safe_width(&inline)? <= width {
        style.push(output, TextStyle::Accent, title);
        output.push_str("  ");
        style.push(output, TextStyle::Bold, target);
        output.push('\n');
        return Ok(());
    }
    for line in wrap(title, width)? {
        style.push(output, TextStyle::Accent, &line);
        output.push('\n');
    }
    for line in wrap(target, width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn push_section_heading(
    output: &mut String,
    heading: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    for line in wrap(heading, width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn push_change(
    output: &mut String,
    action: PlanAction,
    label: &str,
    detail: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    const PREFIX_WIDTH: usize = 2;
    if width <= PREFIX_WIDTH {
        style.push(output, action.text_style(), action.marker());
        output.push('\n');
        for line in wrap(label, width)? {
            style.push(output, TextStyle::Bold, &line);
            output.push('\n');
        }
    } else {
        for (index, line) in wrap(label, width - PREFIX_WIDTH)?.iter().enumerate() {
            if index == 0 {
                style.push(output, action.text_style(), action.marker());
                output.push(' ');
            } else {
                output.push_str("  ");
            }
            style.push(output, TextStyle::Bold, line);
            output.push('\n');
        }
    }
    let detail_indent = 2_usize;
    if width <= detail_indent {
        for line in wrap(detail, width)? {
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
    } else {
        for line in wrap(detail, width - detail_indent)? {
            output.push_str("  ");
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
    }
    Ok(())
}

pub(crate) fn push_detail_field(
    output: &mut String,
    label: &str,
    value: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline_prefix = FIELD_INDENT + FIELD_LABEL_WIDTH;
    if safe_width(label)? <= FIELD_LABEL_WIDTH && width > inline_prefix {
        let prefix = format!("  {label:<FIELD_LABEL_WIDTH$}");
        for (index, line) in wrap(value, width - inline_prefix)?.iter().enumerate() {
            if index == 0 {
                style.push(output, TextStyle::Muted, &prefix);
            } else {
                style.push(output, TextStyle::Muted, &" ".repeat(inline_prefix));
            }
            style.push(output, TextStyle::Muted, line);
            output.push('\n');
        }
    } else {
        let indent = FIELD_INDENT.min(width.saturating_sub(1));
        let content_width = width.saturating_sub(indent).max(1);
        for line in wrap(label, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
        for line in wrap(value, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
    }
    Ok(())
}

pub(crate) fn push_model_list_field(
    output: &mut String,
    label: &str,
    values: &[&str],
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let displayed = values
        .iter()
        .map(|value| display_model_item(value))
        .collect::<Vec<_>>();
    let displayed = displayed.iter().map(String::as_str).collect::<Vec<_>>();
    let inline_prefix = FIELD_INDENT + FIELD_LABEL_WIDTH;
    let inline_width = width.saturating_sub(inline_prefix);
    let every_item_fits = displayed.iter().enumerate().all(|(index, value)| {
        safe_width(value).is_ok_and(|item_width| {
            item_width + usize::from(index + 1 < displayed.len()) <= inline_width
        })
    });
    if safe_width(label)? <= FIELD_LABEL_WIDTH && width > inline_prefix && every_item_fits {
        let prefix = format!("  {label:<FIELD_LABEL_WIDTH$}");
        let continuation = " ".repeat(inline_prefix);
        for (index, line) in wrap_list(&displayed, inline_width)?.iter().enumerate() {
            style.push(
                output,
                TextStyle::Muted,
                if index == 0 { &prefix } else { &continuation },
            );
            style.push(output, TextStyle::Muted, line);
            output.push('\n');
        }
    } else {
        let indent = FIELD_INDENT.min(width.saturating_sub(1));
        let content_width = width.saturating_sub(indent).max(1);
        for line in wrap(label, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
        for value in displayed {
            push_bullet(output, value, width, style)?;
        }
    }
    Ok(())
}

pub(crate) fn display_model_item(model: &str) -> String {
    if model.chars().any(|character| {
        character == ',' || character == '"' || character == '\\' || character.is_whitespace()
    }) {
        serde_json::to_string(model).expect("serializing a model ID string cannot fail")
    } else {
        model.to_owned()
    }
}

pub(crate) fn escape_remote_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if matches!(byte, 0x20..=0x7e) && !matches!(byte, b'"' | b'\\') {
            escaped.push(char::from(byte));
        } else {
            write!(escaped, "\\x{byte:02X}").expect("formatting into a String cannot fail");
        }
    }
    escaped
}

fn wrap_list(values: &[&str], width: usize) -> Result<Vec<String>, PresentationError> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0_usize;
    for (index, value) in values.iter().enumerate() {
        let item = if index + 1 == values.len() {
            (*value).to_owned()
        } else {
            format!("{value},")
        };
        let item_width = safe_width(&item)?;
        let separator_width = usize::from(!line.is_empty());
        if !line.is_empty() && used + separator_width + item_width > width {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if item_width <= width {
            if !line.is_empty() {
                line.push(' ');
                used += 1;
            }
            line.push_str(&item);
            used += item_width;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        let mut wrapped = wrap(&item, width)?;
        line = wrapped
            .pop()
            .expect("wrap always returns at least one line");
        used = safe_width(&line)?;
        lines.extend(wrapped);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    Ok(lines)
}

pub(crate) fn push_plan_summary(
    output: &mut String,
    counts: &PlanCounts,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    for line in wrap(&counts.sentence(), width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn push_bullet(
    output: &mut String,
    value: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    const PREFIX: &str = "  • ";
    const PREFIX_WIDTH: usize = 4;
    if width <= PREFIX_WIDTH {
        for line in wrap(value, width)? {
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
        return Ok(());
    }
    for (index, line) in wrap(value, width.saturating_sub(PREFIX_WIDTH).max(1))?
        .into_iter()
        .enumerate()
    {
        style.push(
            output,
            TextStyle::Muted,
            if index == 0 { PREFIX } else { "    " },
        );
        style.push(output, TextStyle::Muted, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn wrap(value: &str, width: usize) -> Result<Vec<String>, PresentationError> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0_usize;
    for grapheme in value.graphemes(true) {
        let grapheme_width = usize::from(Grapheme::try_from(grapheme)?.width().get());
        if grapheme_width > width {
            return Err(PresentationError::GraphemeExceedsWidth {
                grapheme_width,
                width,
            });
        }
        if !line.is_empty() && used + grapheme_width > width {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push_str(grapheme);
        used += grapheme_width;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    Ok(lines)
}

pub(crate) fn safe_width(value: &str) -> Result<usize, PresentationError> {
    value.graphemes(true).try_fold(0_usize, |width, text| {
        let grapheme = Grapheme::try_from(text)?;
        Ok(width + usize::from(grapheme.width().get()))
    })
}

pub(crate) fn widest_grapheme(value: &str) -> Result<usize, PresentationError> {
    value.graphemes(true).try_fold(0_usize, |width, text| {
        let grapheme = Grapheme::try_from(text)?;
        Ok(width.max(usize::from(grapheme.width().get())))
    })
}

pub(crate) fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

fn readable_number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

pub(crate) fn trim_trailing_newline(output: &mut String) {
    while output.ends_with('\n') {
        output.pop();
    }
}
