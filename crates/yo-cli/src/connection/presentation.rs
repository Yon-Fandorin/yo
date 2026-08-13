use std::{collections::BTreeSet, env, fmt::Write as _, io::IsTerminal as _, num::NonZeroU16};

use unicode_segmentation::UnicodeSegmentation;
use yo_core::{CompleteModelBinding, CredentialMutationAction, ModelSelection};
use yo_tui::surface::{Grapheme, GraphemeError};

const DEFAULT_WIDTH: u16 = 80;
const FIELD_LABEL_WIDTH: usize = 16;
const FIELD_INDENT: usize = 2;

const ANSI_BOLD: &str = "\u{1b}[1m";
const ANSI_BOLD_CYAN: &str = "\u{1b}[1;36m";
const ANSI_GREEN: &str = "\u{1b}[32m";
const ANSI_YELLOW: &str = "\u{1b}[33m";
const ANSI_RED: &str = "\u{1b}[31m";
const ANSI_DIM: &str = "\u{1b}[2m";
const ANSI_RESET: &str = "\u{1b}[0m";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PresentationStyle {
    Plain,
    Ansi,
}

impl PresentationStyle {
    pub(super) fn for_controlling_terminal() -> Self {
        if env::var_os("NO_COLOR").is_some() {
            Self::Plain
        } else {
            Self::Ansi
        }
    }

    fn for_stdout() -> Self {
        if std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none() {
            Self::Ansi
        } else {
            Self::Plain
        }
    }

    fn push(self, output: &mut String, ansi: &str, value: &str) {
        if self == Self::Ansi {
            output.push_str(ansi);
            output.push_str(value);
            output.push_str(ANSI_RESET);
        } else {
            output.push_str(value);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ManagedConnectionChange {
    Create,
    Update,
    Keep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlanAction {
    Add,
    Change,
    Remove,
    Keep,
    Attention,
    Success,
}

impl PlanAction {
    const fn marker(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Change => "~",
            Self::Remove => "−",
            Self::Keep => "=",
            Self::Attention => "!",
            Self::Success => "✓",
        }
    }

    const fn ansi(self) -> &'static str {
        match self {
            Self::Add | Self::Success => ANSI_GREEN,
            Self::Change | Self::Attention => ANSI_YELLOW,
            Self::Remove => ANSI_RED,
            Self::Keep => ANSI_DIM,
        }
    }
}

#[derive(Default)]
struct PlanCounts {
    add: usize,
    change: usize,
    remove: usize,
    keep: usize,
}

impl PlanCounts {
    fn record(&mut self, action: PlanAction) {
        match action {
            PlanAction::Add => self.add += 1,
            PlanAction::Change => self.change += 1,
            PlanAction::Remove => self.remove += 1,
            PlanAction::Keep => self.keep += 1,
            PlanAction::Attention | PlanAction::Success => {},
        }
    }

    fn sentence(&self) -> String {
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
pub(super) struct BindingDetails {
    model: String,
    profile: ProfileDetails,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProfileDetails {
    endpoint: String,
    protocol: String,
    connector: String,
    tokenizer: String,
    input_limit: u64,
    output_limit: u64,
    reasoning: String,
    request_options: String,
    tools: String,
    verification: String,
}

impl BindingDetails {
    fn render(
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
    fn render(
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
                readable_number(self.output_limit)
            ),
            width,
            style,
        )?;
        push_detail_field(output, "Tokenizer", &self.tokenizer, width, style)?;
        push_detail_field(output, "Tools", &self.tools, width, style)?;
        push_detail_field(output, "Verification", &self.verification, width, style)?;
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

struct ProfileGroup<'a> {
    profile: &'a ProfileDetails,
    models: Vec<&'a str>,
}

fn group_profiles(bindings: &[BindingDetails]) -> Vec<ProfileGroup<'_>> {
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
                verification: profile.verification_profile().to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum RemainingBinding {
    Complete { model: String },
    Legacy { model: String },
}

impl RemainingBinding {
    pub(super) fn complete(selection: ModelSelection) -> Self {
        Self::Complete {
            model: selection.model().to_string(),
        }
    }

    pub(super) fn legacy(selection: ModelSelection) -> Self {
        Self::Legacy {
            model: selection.model().to_string(),
        }
    }

    pub(super) fn model(&self) -> &str {
        match self {
            Self::Complete { model } | Self::Legacy { model } => model,
        }
    }

    fn render(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        match self {
            Self::Complete { model } => push_bullet(output, model, width, style),
            Self::Legacy { model } => push_bullet(
                output,
                &format!("{model}  ·  manual legacy profile"),
                width,
                style,
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Confirmation {
    Connect(Box<ConnectPreview>),
    Disconnect(Box<DisconnectPreview>),
}

impl Confirmation {
    #[cfg(test)]
    pub(super) fn render(&self, width: NonZeroU16) -> Result<String, PresentationError> {
        self.render_styled(width, PresentationStyle::Plain)
    }

    pub(super) fn render_styled(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        match self {
            Self::Connect(preview) => preview.render(width, style),
            Self::Disconnect(preview) => preview.render(width, style),
        }
    }

    pub(super) const fn prompt(&self) -> &'static str {
        match self {
            Self::Connect(_) => "Apply this connection plan? [y/N] ",
            Self::Disconnect(_) => "Apply this disconnect plan? [y/N] ",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConnectPreview {
    target: String,
    account: String,
    default_after: String,
    managed_change: ManagedConnectionChange,
    credential_action: CredentialMutationAction,
    default_changed: bool,
    verbose: bool,
    bindings: Vec<BindingDetails>,
}

impl ConnectPreview {
    pub(super) fn new(
        target: String,
        account: String,
        default_after: String,
        managed_change: ManagedConnectionChange,
        credential_action: CredentialMutationAction,
        default_changed: bool,
        bindings: Vec<BindingDetails>,
    ) -> Self {
        Self {
            target,
            account,
            default_after,
            managed_change,
            credential_action,
            default_changed,
            verbose: false,
            bindings,
        }
    }

    pub(super) const fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    fn render(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        let width = usize::from(width.get());
        let mut output = String::new();
        push_title(&mut output, "CONNECT", &self.target, width, style)?;
        output.push('\n');
        push_section_heading(&mut output, "Yo will make these changes:", width, style)?;
        let mut counts = PlanCounts::default();
        let (managed_action, managed_detail) = match self.managed_change {
            ManagedConnectionChange::Create => (PlanAction::Add, format!("Create {}", self.target)),
            ManagedConnectionChange::Update => {
                (PlanAction::Change, format!("Update {}", self.target))
            },
            ManagedConnectionChange::Keep => (PlanAction::Keep, format!("Keep {}", self.target)),
        };
        push_change(
            &mut output,
            managed_action,
            "Managed connection",
            &managed_detail,
            width,
            style,
        )?;
        counts.record(managed_action);
        let verified_models = self
            .bindings
            .iter()
            .map(|binding| binding.model.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let verification_detail = if verified_models.len() == self.bindings.len() {
            format!(
                "verify {} {}",
                verified_models.len(),
                plural(verified_models.len(), "model", "models")
            )
        } else {
            format!(
                "verify {} {} · {} configurations",
                verified_models.len(),
                plural(verified_models.len(), "model", "models"),
                self.bindings.len()
            )
        };
        let (credential_action, credential_detail) = match self.credential_action {
            CredentialMutationAction::Add => (
                PlanAction::Add,
                format!("Save {} · {verification_detail}", self.account),
            ),
            CredentialMutationAction::Replace => (
                PlanAction::Change,
                format!("Replace {} · {verification_detail}", self.account),
            ),
            CredentialMutationAction::Remove => {
                return Err(PresentationError::InvalidPlan(
                    "connect cannot prepare credential removal",
                ));
            },
        };
        push_change(
            &mut output,
            credential_action,
            "API key",
            &credential_detail,
            width,
            style,
        )?;
        push_model_list_field(&mut output, "Models", &verified_models, width, style)?;
        counts.record(credential_action);
        let default_action = if self.default_changed {
            PlanAction::Change
        } else {
            PlanAction::Keep
        };
        push_change(
            &mut output,
            default_action,
            "Default model",
            &self.default_after,
            width,
            style,
        )?;
        counts.record(default_action);
        if self.verbose {
            output.push('\n');
            let groups = group_profiles(&self.bindings);
            let multiple = groups.len() > 1;
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                let heading = if multiple {
                    format!("Connection profile {} of {}", index + 1, groups.len())
                } else {
                    "Connection profile".to_owned()
                };
                push_section_heading(&mut output, &heading, width, style)?;
                push_model_list_field(
                    &mut output,
                    &format!("Models ({})", group.models.len()),
                    &group.models,
                    width,
                    style,
                )?;
                group.profile.render(&mut output, width, style)?;
            }
        }
        output.push('\n');
        push_plan_summary(&mut output, &counts, width, style)?;
        trim_trailing_newline(&mut output);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisconnectPreview {
    target: String,
    managed_source: String,
    removed: BindingDetails,
    impact: DisconnectImpact,
    remaining: Vec<RemainingBinding>,
    verbose: bool,
}

impl DisconnectPreview {
    pub(super) fn new(
        target: String,
        managed_source: String,
        removed: BindingDetails,
        impact: DisconnectImpact,
        remaining: Vec<RemainingBinding>,
        verbose: bool,
    ) -> Self {
        Self {
            target,
            managed_source,
            removed,
            impact,
            remaining,
            verbose,
        }
    }

    fn render(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        let width = usize::from(width.get());
        let mut output = String::new();
        push_title(&mut output, "DISCONNECT", &self.target, width, style)?;
        output.push('\n');
        push_section_heading(&mut output, "Yo will make these changes:", width, style)?;
        let mut counts = PlanCounts::default();
        push_change(
            &mut output,
            PlanAction::Remove,
            "Managed connection",
            &format!("Remove {}", self.target),
            width,
            style,
        )?;
        counts.record(PlanAction::Remove);
        push_change(
            &mut output,
            self.impact.default.action,
            "Default model",
            &self.impact.default.detail,
            width,
            style,
        )?;
        counts.record(self.impact.default.action);
        push_change(
            &mut output,
            self.impact.api_key.action,
            "API key",
            &self.impact.api_key.detail,
            width,
            style,
        )?;
        counts.record(self.impact.api_key.action);
        push_change(
            &mut output,
            self.impact.new_sessions.action,
            "New sessions",
            &self.impact.new_sessions.detail,
            width,
            style,
        )?;
        push_change(
            &mut output,
            self.impact.saved_sessions.action,
            "Saved sessions",
            &self.impact.saved_sessions.detail,
            width,
            style,
        )?;
        if self.verbose {
            output.push('\n');
            push_section_heading(&mut output, "Connection being removed", width, style)?;
            push_detail_field(&mut output, "Source", &self.managed_source, width, style)?;
            self.removed.render(&mut output, width, style)?;
            output.push('\n');
            push_section_heading(
                &mut output,
                &format!(
                    "Still available for this account ({})",
                    self.remaining.len()
                ),
                width,
                style,
            )?;
            if self.remaining.is_empty() {
                push_bullet(&mut output, "None", width, style)?;
            } else {
                for (index, binding) in self.remaining.iter().enumerate() {
                    if index > 0 {
                        output.push('\n');
                    }
                    binding.render(&mut output, width, style)?;
                }
            }
        }
        output.push('\n');
        push_plan_summary(&mut output, &counts, width, style)?;
        trim_trailing_newline(&mut output);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisconnectImpact {
    default: DisconnectEffect,
    api_key: DisconnectEffect,
    new_sessions: DisconnectEffect,
    saved_sessions: DisconnectEffect,
}

impl DisconnectImpact {
    pub(super) fn new(
        default: DisconnectEffect,
        api_key: DisconnectEffect,
        new_sessions: DisconnectEffect,
        saved_sessions: DisconnectEffect,
    ) -> Self {
        Self {
            default,
            api_key,
            new_sessions,
            saved_sessions,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisconnectEffect {
    detail: String,
    action: PlanAction,
}

impl DisconnectEffect {
    pub(super) fn change(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Change,
        }
    }

    pub(super) fn remove(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Remove,
        }
    }

    pub(super) fn keep(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Keep,
        }
    }

    pub(super) fn ready(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Success,
        }
    }

    pub(super) fn attention(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Attention,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PresentationError {
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

pub(super) fn default_width() -> NonZeroU16 {
    NonZeroU16::new(DEFAULT_WIDTH).expect("the default terminal width is nonzero")
}

pub(super) fn connect_success(target: &str, verified: usize, default: &str) -> String {
    let style = PresentationStyle::for_stdout();
    let mut output = String::new();
    style.push(&mut output, ANSI_GREEN, "✓");
    output.push(' ');
    style.push(&mut output, ANSI_BOLD, "Connected");
    write!(
        output,
        "\n\n  Model     {target}\n  Verified  {verified} model {}\n  Default   {default}\n",
        plural(verified, "profile", "profiles")
    )
    .expect("writing to String cannot fail");
    output
}

pub(super) fn disconnect_success(target: &str, api_key: &str, default: &str) -> String {
    let style = PresentationStyle::for_stdout();
    let mut output = String::new();
    style.push(&mut output, ANSI_GREEN, "✓");
    output.push(' ');
    style.push(&mut output, ANSI_BOLD, "Disconnected");
    write!(
        output,
        "\n\n  Model    {target}\n  API key  {api_key}\n  Default  {default}\n"
    )
    .expect("writing to String cannot fail");
    output
}

fn push_title(
    output: &mut String,
    title: &str,
    target: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline = format!("{title}  {target}");
    if safe_width(&inline)? <= width {
        style.push(output, ANSI_BOLD_CYAN, title);
        output.push_str("  ");
        style.push(output, ANSI_BOLD, target);
        output.push('\n');
        return Ok(());
    }
    for line in wrap(title, width)? {
        style.push(output, ANSI_BOLD_CYAN, &line);
        output.push('\n');
    }
    for line in wrap(target, width)? {
        style.push(output, ANSI_BOLD, &line);
        output.push('\n');
    }
    Ok(())
}

fn push_section_heading(
    output: &mut String,
    heading: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    for line in wrap(heading, width)? {
        style.push(output, ANSI_BOLD, &line);
        output.push('\n');
    }
    Ok(())
}

fn push_change(
    output: &mut String,
    action: PlanAction,
    label: &str,
    detail: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    const PREFIX_WIDTH: usize = 2;
    if width <= PREFIX_WIDTH {
        style.push(output, action.ansi(), action.marker());
        output.push('\n');
        for line in wrap(label, width)? {
            style.push(output, ANSI_BOLD, &line);
            output.push('\n');
        }
    } else {
        for (index, line) in wrap(label, width - PREFIX_WIDTH)?.iter().enumerate() {
            if index == 0 {
                style.push(output, action.ansi(), action.marker());
                output.push(' ');
            } else {
                output.push_str("  ");
            }
            style.push(output, ANSI_BOLD, line);
            output.push('\n');
        }
    }
    let detail_indent = 2_usize;
    if width <= detail_indent {
        for line in wrap(detail, width)? {
            style.push(output, ANSI_DIM, &line);
            output.push('\n');
        }
    } else {
        for line in wrap(detail, width - detail_indent)? {
            output.push_str("  ");
            style.push(output, ANSI_DIM, &line);
            output.push('\n');
        }
    }
    Ok(())
}

fn push_detail_field(
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
                style.push(output, ANSI_DIM, &prefix);
            } else {
                style.push(output, ANSI_DIM, &" ".repeat(inline_prefix));
            }
            style.push(output, ANSI_DIM, line);
            output.push('\n');
        }
    } else {
        let indent = FIELD_INDENT.min(width.saturating_sub(1));
        let content_width = width.saturating_sub(indent).max(1);
        for line in wrap(label, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, ANSI_DIM, &line);
            output.push('\n');
        }
        for line in wrap(value, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, ANSI_DIM, &line);
            output.push('\n');
        }
    }
    Ok(())
}

fn push_model_list_field(
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
                ANSI_DIM,
                if index == 0 { &prefix } else { &continuation },
            );
            style.push(output, ANSI_DIM, line);
            output.push('\n');
        }
    } else {
        let indent = FIELD_INDENT.min(width.saturating_sub(1));
        let content_width = width.saturating_sub(indent).max(1);
        for line in wrap(label, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, ANSI_DIM, &line);
            output.push('\n');
        }
        for value in displayed {
            push_bullet(output, value, width, style)?;
        }
    }
    Ok(())
}

pub(super) fn display_model_item(model: &str) -> String {
    if model.chars().any(|character| {
        character == ',' || character == '"' || character == '\\' || character.is_whitespace()
    }) {
        serde_json::to_string(model).expect("serializing a model ID string cannot fail")
    } else {
        model.to_owned()
    }
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

fn push_plan_summary(
    output: &mut String,
    counts: &PlanCounts,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    for line in wrap(&counts.sentence(), width)? {
        style.push(output, ANSI_BOLD, &line);
        output.push('\n');
    }
    Ok(())
}

fn push_bullet(
    output: &mut String,
    value: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    const PREFIX: &str = "  • ";
    const PREFIX_WIDTH: usize = 4;
    if width <= PREFIX_WIDTH {
        for line in wrap(value, width)? {
            style.push(output, ANSI_DIM, &line);
            output.push('\n');
        }
        return Ok(());
    }
    for (index, line) in wrap(value, width.saturating_sub(PREFIX_WIDTH).max(1))?
        .into_iter()
        .enumerate()
    {
        style.push(output, ANSI_DIM, if index == 0 { PREFIX } else { "    " });
        style.push(output, ANSI_DIM, &line);
        output.push('\n');
    }
    Ok(())
}

fn wrap(value: &str, width: usize) -> Result<Vec<String>, PresentationError> {
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

fn safe_width(value: &str) -> Result<usize, PresentationError> {
    value.graphemes(true).try_fold(0_usize, |width, text| {
        let grapheme = Grapheme::try_from(text)?;
        Ok(width + usize::from(grapheme.width().get()))
    })
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
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

fn trim_trailing_newline(output: &mut String) {
    while output.ends_with('\n') {
        output.pop();
    }
}

#[cfg(test)]
mod tests {
    use yo_tui::surface::cell_width;

    use super::*;

    fn width(value: u16) -> NonZeroU16 {
        NonZeroU16::new(value).unwrap()
    }

    fn fixture_binding() -> BindingDetails {
        fixture_binding_for_model("alpha")
    }

    fn fixture_binding_for_model(model: &str) -> BindingDetails {
        let mut durable = serde_json::from_str::<serde_json::Value>(
            r#"{"provider":"vendor","account":"team","model":"alpha","connector":"openai-responses","base_url":"https://long-provider.example.test/compatible-mode/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":4096,"max_output_tokens":128,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
        )
        .unwrap();
        durable["model"] = serde_json::Value::String(model.to_owned());
        BindingDetails::from(
            &CompleteModelBinding::from_durable_json(&durable.to_string()).unwrap(),
        )
    }

    // 기본 confirmation은 적용 판단에 필요한 change set과 요약만 보여 주며, exact profile은
    // -v를 선택한 경우에만 노출해 반복 실행의 기본 화면을 짧게 유지합니다.
    #[test]
    fn compact_connect_preview_hides_exact_profile_until_verbose() {
        let preview = Confirmation::Connect(Box::new(ConnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "vendor:team".to_owned(),
            "unset  →  vendor:team:alpha".to_owned(),
            ManagedConnectionChange::Create,
            CredentialMutationAction::Add,
            true,
            vec![fixture_binding()],
        )));

        let output = preview.render(width(80)).unwrap();

        assert!(output.contains("Yo will make these changes:\n+ Managed connection"));
        assert!(output.contains("+ API key\n  Save vendor:team · verify 1 model"));
        assert!(output.contains("  Models          alpha"));
        assert!(output.contains("~ Default model\n  unset  →  vendor:team:alpha"));
        assert!(!output.contains("Connection profile"));
        assert!(!output.contains("long-provider.example.test"));
        assert!(output.ends_with("Plan: 2 to add, 1 to change."));
    }

    // 80열 connect 확인 화면은 먼저 사람이 결정할 대상·API key·default를 보여 주고,
    // exact profile 세부정보를 이름 있는 보조 영역에 배치합니다.
    #[test]
    fn connect_preview_prioritizes_the_decision_before_exact_details() {
        let preview = Confirmation::Connect(Box::new(
            ConnectPreview::new(
                "vendor:team:alpha".to_owned(),
                "vendor:team".to_owned(),
                "unset  →  vendor:team:alpha".to_owned(),
                ManagedConnectionChange::Create,
                CredentialMutationAction::Add,
                true,
                vec![fixture_binding()],
            )
            .with_verbose(true),
        ));

        let output = preview.render(width(80)).unwrap();

        assert!(output.starts_with("CONNECT  vendor:team:alpha"));
        assert!(output.contains("Yo will make these changes:\n+ Managed connection"));
        assert!(output.contains("+ API key\n  Save vendor:team"));
        assert!(output.contains("Connection profile"));
        assert!(output.contains("  Models (1)      alpha"));
        assert!(output.contains("  Endpoint"));
        assert!(output.contains("  Request options"));
        assert!(
            output.find("~ Default model").unwrap() < output.find("Connection profile").unwrap()
        );
        assert!(output.ends_with("Plan: 2 to add, 1 to change."));
    }

    // 여러 모델 목록은 쉼표 경계에서만 다음 줄로 넘겨 충분한 폭이 있는데도 긴 모델 ID를
    // 중간에서 자르지 않으며, 각 모델을 정확히 한 번씩 그대로 보여 줍니다.
    #[test]
    fn model_list_wraps_between_models_without_splitting_identifiers() {
        let models = [
            "anthropic/claude-opus-4",
            "deepseek/deepseek-r1",
            "mistralai/mistral-large",
        ];

        let lines = wrap_list(&models, 30).unwrap();

        assert_eq!(
            lines,
            [
                "anthropic/claude-opus-4,",
                "deepseek/deepseek-r1,",
                "mistralai/mistral-large",
            ]
        );
    }

    // 쉼표나 내부 공백이 허용된 Model ID는 일반 ID와 섞여도 따옴표로 경계를 보존하며,
    // compact와 verbose 목록이 같은 두 모델을 서로 다른 항목으로 표시합니다.
    #[test]
    fn model_lists_quote_delimiter_bearing_identifiers_in_both_views() {
        let render = |verbose| {
            Confirmation::Connect(Box::new(
                ConnectPreview::new(
                    "vendor:team:a".to_owned(),
                    "vendor:team".to_owned(),
                    "unset  →  vendor:team:a".to_owned(),
                    ManagedConnectionChange::Create,
                    CredentialMutationAction::Add,
                    true,
                    vec![
                        fixture_binding_for_model("a"),
                        fixture_binding_for_model("b, c"),
                    ],
                )
                .with_verbose(verbose),
            ))
            .render(width(80))
            .unwrap()
        };

        let compact = render(false);
        let verbose = render(true);

        assert!(compact.contains("Models          a, \"b, c\""));
        assert_eq!(verbose.matches("Models (2)      a, \"b, c\"").count(), 1);
    }

    // Model ID 자체는 inline 폭에 맞지만 뒤 구분자까지 맞지 않으면 쉼표를 별도 줄로
    // 떼거나 ID를 자르지 않고 명확한 bullet 항목으로 전환합니다.
    #[test]
    fn model_list_uses_bullets_when_an_inline_separator_would_not_fit() {
        let mut output = String::new();

        push_model_list_field(
            &mut output,
            "Models",
            &["abcde", "f"],
            23,
            PresentationStyle::Plain,
        )
        .unwrap();

        assert_eq!(output, "  Models\n  • abcde\n  • f\n");
    }

    // 좁은 terminal에서도 long endpoint와 versioned profile을 Yo가 직접 grapheme 단위로
    // 감싸 모든 물리 줄을 폭 안에 두며, 정보 손실 없이 다시 이어 읽을 수 있습니다.
    #[test]
    fn narrow_preview_wraps_every_line_without_losing_exact_values() {
        let binding = fixture_binding();
        let preview = Confirmation::Connect(Box::new(
            ConnectPreview::new(
                "vendor:team:alpha".to_owned(),
                "vendor:team".to_owned(),
                "unset  →  vendor:team:alpha".to_owned(),
                ManagedConnectionChange::Create,
                CredentialMutationAction::Add,
                true,
                vec![binding],
            )
            .with_verbose(true),
        ));

        let output = preview.render(width(36)).unwrap();

        for line in output.lines() {
            assert!(
                cell_width(line).unwrap() <= 36,
                "overwide connection-preview line: {line:?}"
            );
        }
        let compact = output.split_whitespace().collect::<String>();
        assert!(compact.contains("https://long-provider.example.test/compatible-mode/v1"));
        assert!(compact.contains("utf8-bytes/v1"));
        assert!(compact.contains("semantic-terminal/v1"));
    }

    // disconnect 화면은 실제 변화와 Session 영향이 제거 상세보다 먼저 나오고, 남는 모델은
    // 식별에 필요한 reference만 보여 제거 profile 전체를 반복하지 않습니다.
    #[test]
    fn disconnect_preview_prioritizes_effects_and_compacts_remaining_models() {
        let removed = fixture_binding();
        let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "Managed copy removed; equal manual configuration remains".to_owned(),
            removed,
            DisconnectImpact::new(
                DisconnectEffect::change("Clear vendor:team:alpha".to_owned()),
                DisconnectEffect::keep(
                    "Keep it because another configured model still uses vendor:team".to_owned(),
                ),
                DisconnectEffect::attention(
                    "Need another available model because the default will be cleared".to_owned(),
                ),
                DisconnectEffect::ready(
                    "Can resume through the equal manual configuration; history is kept".to_owned(),
                ),
            ),
            vec![RemainingBinding::Complete {
                model: "alpha".to_owned(),
            }],
            true,
        )));

        let output = preview.render(width(80)).unwrap();

        assert!(output.starts_with("DISCONNECT  vendor:team:alpha"));
        assert!(
            output.find("= API key").unwrap() < output.find("Connection being removed").unwrap()
        );
        assert_eq!(
            output.matches("https://long-provider.example.test").count(),
            1
        );
        assert!(output.contains("Still available for this account (1)\n  • alpha"));
    }

    // disconnect의 문장형 risk와 exact endpoint도 좁은 폭에서 셸의 임의 개행에 의존하지
    // 않고 모든 줄이 폭 안에 남습니다.
    #[test]
    fn narrow_disconnect_preview_keeps_every_line_within_width() {
        let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "Managed connection only; no manual configuration remains for this model".to_owned(),
            fixture_binding(),
            DisconnectImpact::new(
                DisconnectEffect::change("Clear vendor:team:alpha".to_owned()),
                DisconnectEffect::remove(
                    "Remove it because no configured model still uses vendor:team".to_owned(),
                ),
                DisconnectEffect::attention(
                    "Need another available model because the default will be cleared".to_owned(),
                ),
                DisconnectEffect::attention(
                    "May not resume until this exact model is restored; history is kept".to_owned(),
                ),
            ),
            Vec::new(),
            true,
        )));

        let output = preview.render(width(36)).unwrap();

        for line in output.lines() {
            assert!(
                cell_width(line).unwrap() <= 36,
                "overwide disconnect-preview line: {line:?}"
            );
        }
        assert!(output.contains("Still available for this account (0)"));
    }

    // structured parameter의 연속 공백도 의미 있는 문자열 bytes일 수 있으므로 wrapping 전후
    // 조각을 이어 붙이면 원문과 같고 split_whitespace식 정규화가 일어나지 않습니다.
    #[test]
    fn wrapping_preserves_exact_whitespace_in_profile_values() {
        let original = r#"{"note":"a  b"}"#;
        let wrapped = wrap(original, 7).unwrap();

        assert_eq!(wrapped.concat(), original);
        assert!(wrapped.iter().all(|line| cell_width(line).unwrap() <= 7));
    }

    // ANSI 장식은 TTY 전용 styled 경로에만 들어가고 같은 preview의 평문 경로는 로그와
    // snapshot 비교에 안전한 의미 marker를 그대로 유지합니다.
    #[test]
    fn ansi_style_decorates_semantics_without_changing_plain_output() {
        let preview = Confirmation::Connect(Box::new(ConnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "vendor:team".to_owned(),
            "unset  →  vendor:team:alpha".to_owned(),
            ManagedConnectionChange::Create,
            CredentialMutationAction::Replace,
            true,
            vec![fixture_binding()],
        )));

        let plain = preview.render(width(80)).unwrap();
        let styled = preview
            .render_styled(width(80), PresentationStyle::Ansi)
            .unwrap();

        assert!(!plain.contains('\u{1b}'));
        assert!(styled.contains("\u{1b}[1;36mCONNECT\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[33m~\u{1b}[0m"));
        assert_eq!(strip_ansi(&styled), plain);
    }

    // 제목보다도 좁은 1~15열 terminal에서 제목과 모든 본문을 자체 줄바꿈해 ANSI를
    // 제외한 각 물리 줄이 관찰한 폭을 넘지 않습니다.
    #[test]
    fn every_heading_fits_terminals_narrower_than_the_title() {
        let preview = Confirmation::Connect(Box::new(
            ConnectPreview::new(
                "vendor:team:alpha".to_owned(),
                "vendor:team".to_owned(),
                "unset  →  vendor:team:alpha".to_owned(),
                ManagedConnectionChange::Create,
                CredentialMutationAction::Add,
                true,
                vec![fixture_binding()],
            )
            .with_verbose(true),
        ));

        for columns in 1..=15 {
            let output = preview.render(width(columns)).unwrap();
            for line in output.lines() {
                assert!(
                    cell_width(line).unwrap() <= usize::from(columns),
                    "{columns}-cell terminal received overwide line {line:?}"
                );
            }
        }
    }

    // 독립 default-ignorable grapheme은 폭 0이라 보이지 않는 내용을 confirmation에
    // 몰래 섞을 수 있으므로 일반 문자처럼 허용하지 않고 terminal-safe 경계에서 거절합니다.
    #[test]
    fn wrapping_rejects_an_isolated_zero_width_grapheme() {
        assert!(matches!(
            wrap("\u{200b}", 80),
            Err(PresentationError::UnsafeText(GraphemeError::ZeroWidth))
        ));
    }

    fn strip_ansi(value: &str) -> String {
        let mut output = String::new();
        let mut characters = value.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\u{1b}' && characters.peek() == Some(&'[') {
                characters.next();
                for next in characters.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                output.push(character);
            }
        }
        output
    }
}
