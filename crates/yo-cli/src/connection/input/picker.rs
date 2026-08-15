use std::fs::File;

use super::super::presentation::{PresentationStyle, default_width, escape_remote_text};
use crate::AppError;

const MAX_VISIBLE_RESULTS: usize = 8;
const MAX_QUERY_BYTES: usize = 1024;

mod terminal;

use self::terminal::{PickerKey, PickerTerminalScope, read_key};

pub(super) fn select_model(
    terminal: &mut File,
    models: &[ModelPickerItem],
    style: PresentationStyle,
) -> Result<Option<usize>, AppError> {
    if models.is_empty() {
        return Err(AppError::message(
            "model inventory contains no valid ModelId",
        ));
    }
    let identity = PickerIdentity::from_models(models)?;
    let choices = models.iter().map(PickerChoice::from).collect::<Vec<_>>();
    let mut state = PickerState::new(&choices);
    let mut scope = PickerTerminalScope::enter(terminal)?;
    loop {
        scope.render(&identity, &state, &choices, style)?;
        match read_key(terminal)? {
            PickerKey::Up => state.move_up(),
            PickerKey::Down => state.move_down(),
            PickerKey::Backspace => state.pop_query(&choices),
            PickerKey::Text(value) => state.push_query(&value, &choices),
            PickerKey::Enter => {
                if let Some(selected) = state.accept_selected(&choices) {
                    scope.finish()?;
                    return Ok(Some(selected));
                }
            },
            PickerKey::Cancel => {
                scope.finish()?;
                return Ok(None);
            },
            PickerKey::Ignore => {},
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::connection) struct ModelPickerItem {
    provider: String,
    account: String,
    display_name: String,
    model_id: String,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
    tool_policy: Option<String>,
    reasoning: Option<bool>,
    enabled: bool,
    disabled_reason: Option<String>,
}

impl ModelPickerItem {
    pub(in crate::connection) fn from_openrouter(
        model: &yo_core::OpenRouterDiscoveredModel,
    ) -> Self {
        let disabled_reason = match model.availability() {
            yo_core::OpenRouterModelAvailability::Enabled => None,
            yo_core::OpenRouterModelAvailability::Disabled(reason) => {
                Some(reason.as_str().to_owned())
            },
        };
        Self {
            provider: model.provider().as_str().to_owned(),
            account: model.account().as_str().to_owned(),
            display_name: model.display_name().to_owned(),
            model_id: model.model_id().as_str().to_owned(),
            input_limit: model.input_limit(),
            output_limit: model.output_limit(),
            tool_policy: model
                .effective_tool_policy()
                .map(|policy| policy.as_str().to_owned()),
            reasoning: model.reasoning(),
            enabled: model.is_enabled(),
            disabled_reason,
        }
    }

    pub(in crate::connection) fn from_qwencloud(model: &yo_core::QwenCloudCatalogModel) -> Self {
        let disabled_reason = match model.availability() {
            yo_core::QwenCloudCatalogAvailability::Enabled => None,
            yo_core::QwenCloudCatalogAvailability::Disabled(reason) => {
                Some(reason.as_str().to_owned())
            },
        };
        Self {
            provider: model.provider().as_str().to_owned(),
            account: model.account().as_str().to_owned(),
            display_name: model.display_name().to_owned(),
            model_id: model.model_id().as_str().to_owned(),
            input_limit: model.input_limit(),
            output_limit: model.output_limit(),
            tool_policy: model.tool_policy().map(str::to_owned),
            reasoning: model.reasoning(),
            enabled: model.is_enabled(),
            disabled_reason,
        }
    }

    #[cfg(test)]
    pub(in crate::connection) fn model_id(&self) -> &str {
        &self.model_id
    }

    #[cfg(test)]
    pub(in crate::connection) const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[cfg(test)]
    pub(in crate::connection) fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }
}

#[derive(Debug)]
struct PickerIdentity {
    provider: String,
    account: String,
}

impl PickerIdentity {
    fn from_models(models: &[ModelPickerItem]) -> Result<Self, AppError> {
        let first = &models[0];
        if models
            .iter()
            .any(|model| model.provider != first.provider || model.account != first.account)
        {
            return Err(AppError::message(
                "model picker items must belong to one Provider and Account",
            ));
        }
        Ok(Self {
            provider: first.provider.clone(),
            account: first.account.clone(),
        })
    }
}

#[derive(Clone, Debug)]
struct PickerChoice {
    display_name: String,
    model_id: String,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
    tool_policy: Option<String>,
    reasoning: Option<bool>,
    enabled: bool,
    disabled_reason: Option<String>,
}

impl From<&ModelPickerItem> for PickerChoice {
    fn from(model: &ModelPickerItem) -> Self {
        Self {
            display_name: model.display_name.clone(),
            model_id: model.model_id.clone(),
            input_limit: model.input_limit,
            output_limit: model.output_limit,
            tool_policy: model.tool_policy.clone(),
            reasoning: model.reasoning,
            enabled: model.enabled,
            disabled_reason: model.disabled_reason.clone(),
        }
    }
}

#[derive(Debug)]
struct PickerState {
    query: String,
    matches: Vec<usize>,
    selected: Option<usize>,
    viewport_start: usize,
    disabled_notice: Option<String>,
}

impl PickerState {
    fn new(choices: &[PickerChoice]) -> Self {
        let mut state = Self {
            query: String::new(),
            matches: Vec::new(),
            selected: None,
            viewport_start: 0,
            disabled_notice: None,
        };
        state.recompute(choices);
        state
    }

    fn recompute(&mut self, choices: &[PickerChoice]) {
        let query = yo_core::normalized_search_key(&self.query);
        self.matches = choices
            .iter()
            .enumerate()
            .filter_map(|(index, choice)| {
                (query.is_empty()
                    || yo_core::normalized_search_key(&choice.display_name).contains(&query)
                    || yo_core::normalized_search_key(&choice.model_id).contains(&query))
                .then_some(index)
            })
            .collect();
        self.selected = (!self.matches.is_empty()).then_some(0);
        self.viewport_start = 0;
        self.disabled_notice = None;
    }

    fn push_query(&mut self, value: &str, choices: &[PickerChoice]) {
        if self.query.len().saturating_add(value.len()) <= MAX_QUERY_BYTES {
            self.query.push_str(value);
            self.recompute(choices);
        }
    }

    fn pop_query(&mut self, choices: &[PickerChoice]) {
        if self.query.pop().is_some() {
            self.recompute(choices);
        }
    }

    fn move_up(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let selected = selected.saturating_sub(1);
        self.selected = Some(selected);
        self.disabled_notice = None;
        if selected < self.viewport_start {
            self.viewport_start = selected;
        }
    }

    fn move_down(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let selected = (selected + 1).min(self.matches.len() - 1);
        self.selected = Some(selected);
        self.disabled_notice = None;
        if selected >= self.viewport_start + MAX_VISIBLE_RESULTS {
            self.viewport_start = selected + 1 - MAX_VISIBLE_RESULTS;
        }
    }

    fn selected_model_index(&self) -> Option<usize> {
        self.selected.map(|selected| self.matches[selected])
    }

    fn accept_selected(&mut self, choices: &[PickerChoice]) -> Option<usize> {
        let index = self.selected_model_index()?;
        if choices[index].enabled {
            Some(index)
        } else {
            self.disabled_notice = choices[index].disabled_reason.clone();
            None
        }
    }
}

fn render_lines(
    identity: &PickerIdentity,
    state: &PickerState,
    choices: &[PickerChoice],
    width: usize,
    style: PresentationStyle,
) -> Vec<String> {
    let width = width.saturating_sub(1).max(1);
    let mut lines = vec![styled("Model catalog", "\x1b[1;36m", style)];
    lines.extend(wrap_ascii(
        &format!("Provider  {}", escape_remote_text(&identity.provider)),
        width,
    ));
    lines.extend(wrap_ascii(
        &format!("Account  {}", escape_remote_text(&identity.account)),
        width,
    ));
    lines.extend([
        styled("Select one model", "\x1b[1m", style),
        clip_line(
            &format!("Search  {}_", escape_remote_text(&state.query)),
            width,
        ),
        String::new(),
    ]);
    if state.matches.is_empty() {
        lines.push(styled("  No matching models", "\x1b[2m", style));
    } else {
        for match_index in state.viewport_start
            ..(state.viewport_start + MAX_VISIBLE_RESULTS).min(state.matches.len())
        {
            let choice = &choices[state.matches[match_index]];
            let marker = if state.selected == Some(match_index) {
                "›"
            } else {
                " "
            };
            let tools = match choice.tool_policy.as_deref() {
                Some("local-tools/v1") => "tools",
                Some("no-tools/v1") => "no tools",
                Some(_) => "tools unsupported",
                None => "tools ?",
            };
            let reasoning = match choice.reasoning {
                Some(true) => "reasoning",
                Some(false) => "no reasoning",
                None => "reasoning ?",
            };
            let availability = if choice.enabled { "ready" } else { "disabled" };
            let row = format!(
                "{marker} {}  {} ctx · {} out · {tools} · {reasoning} · {availability}",
                escape_remote_text(&choice.display_name),
                readable_optional_limit(choice.input_limit),
                readable_optional_limit(choice.output_limit),
            );
            lines.push(if state.selected == Some(match_index) {
                styled(&clip_line(&row, width), "\x1b[30;46m", style)
            } else {
                clip_line(&row, width)
            });
        }
    }
    lines.push(String::new());
    if let Some(index) = state.selected_model_index() {
        let id = format!("Model  {}", escape_remote_text(&choices[index].model_id));
        lines.extend(wrap_ascii(&id, width));
        let availability = state.disabled_notice.as_deref().map_or_else(
            || {
                if choices[index].enabled {
                    "Availability  ready".to_owned()
                } else {
                    "Availability  disabled · Enter for reason".to_owned()
                }
            },
            |reason| format!("Unavailable  {reason}"),
        );
        lines.extend(wrap_ascii(&availability, width));
    }
    lines.push(styled(
        "↑↓ navigate · type to filter · Enter select · Esc cancel",
        "\x1b[2m",
        style,
    ));
    lines
        .into_iter()
        .map(|line| clip_styled_line(&line, width))
        .collect()
}

fn readable_limit(value: u64) -> String {
    if value.is_multiple_of(1_000) {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

fn readable_optional_limit(value: Option<u64>) -> String {
    value.map_or_else(|| "?".to_owned(), readable_limit)
}

fn styled(value: &str, ansi: &str, style: PresentationStyle) -> String {
    match style {
        PresentationStyle::Plain => value.to_owned(),
        PresentationStyle::Ansi => format!("{ansi}{value}\x1b[0m"),
    }
}

fn clip_line(value: &str, width: usize) -> String {
    if value.len() <= width {
        return value.to_owned();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    format!("{}...", value.chars().take(width - 3).collect::<String>())
}

fn clip_styled_line(value: &str, width: usize) -> String {
    if !value.contains('\x1b') {
        return clip_line(value, width);
    }
    let Some(prefix_end) = value.find('m') else {
        return clip_line(value, width);
    };
    let prefix = &value[..=prefix_end];
    let content = value[prefix_end + 1..]
        .strip_suffix("\x1b[0m")
        .unwrap_or("");
    format!("{prefix}{}\x1b[0m", clip_line(content, width))
}

fn wrap_ascii(value: &str, width: usize) -> Vec<String> {
    value
        .as_bytes()
        .chunks(width.max(1))
        .map(|chunk| String::from_utf8(chunk.to_vec()).expect("escaped picker text is ASCII"))
        .collect()
}

#[cfg(test)]
mod tests;
