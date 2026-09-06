use std::fs::File;

use unicode_segmentation::UnicodeSegmentation;
use yo_tui::surface::Grapheme;

use crate::{
    AppError,
    connection::presentation::{default_width, escape_remote_text},
    presentation::{PresentationStyle, TextStyle},
};

const MAX_VISIBLE_RESULTS: usize = 8;
const MAX_QUERY_BYTES: usize = 1024;

mod terminal;

use self::terminal::{PickerInput, PickerKey, PickerTerminalScope};

pub(crate) fn select_model(
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
    let mut input = PickerInput::new(terminal);
    loop {
        scope.render(&identity, &state, &choices, style)?;
        match input.read_key()? {
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
pub(crate) struct ModelPickerItem {
    provider: String,
    account: String,
    display_name: String,
    model_id: String,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
    tool_policy: Option<String>,
    reasoning: Option<bool>,
    reasoning_label: Option<String>,
    badges: Vec<String>,
    enabled: bool,
    disabled_reason: Option<String>,
}

impl ModelPickerItem {
    pub(crate) fn from_openrouter(model: &yo_core::OpenRouterDiscoveredModel) -> Self {
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
            reasoning_label: None,
            badges: Vec::new(),
            enabled: model.is_enabled(),
            disabled_reason,
        }
    }

    pub(crate) fn from_qwencloud(model: &yo_core::QwenCloudCatalogModel) -> Self {
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
            reasoning_label: None,
            badges: Vec::new(),
            enabled: model.is_enabled(),
            disabled_reason,
        }
    }

    pub(crate) fn from_kimi(model: &yo_core::KimiCatalogModel) -> Self {
        let disabled_reason = match model.availability() {
            yo_core::KimiCatalogAvailability::Enabled => None,
            yo_core::KimiCatalogAvailability::Disabled(reason) => Some(reason.as_str().to_owned()),
        };
        let mut badges = Vec::new();
        if model.recommended() {
            badges.push("recommended".to_owned());
        }
        if model.high_speed() {
            badges.push("high-speed".to_owned());
        }
        let reasoning_label = if model.entry().is_some() {
            match model.model_id().as_str() {
                "kimi-k3" => Some("reasoning required/max".to_owned()),
                "k3" | "k3-256k" => Some("reasoning required/high".to_owned()),
                "kimi-k2.7-code"
                | "kimi-k2.7-code-highspeed"
                | "kimi-for-coding"
                | "kimi-for-coding-highspeed" => Some("reasoning required".to_owned()),
                "kimi-k2.6" => Some(
                    model
                        .reasoning()
                        .map_or("reasoning unknown/off", |available| {
                            if available {
                                "reasoning available/off"
                            } else {
                                "reasoning unavailable/off"
                            }
                        })
                        .to_owned(),
                ),
                _ => generic_reasoning_label(model.reasoning()),
            }
        } else {
            generic_reasoning_label(model.reasoning())
        };
        Self {
            provider: model.provider().as_str().to_owned(),
            account: model.account().as_str().to_owned(),
            display_name: model.display_name().to_owned(),
            model_id: model.model_id().as_str().to_owned(),
            input_limit: model.input_limit(),
            output_limit: model.output_limit(),
            tool_policy: model.entry().map(|_| "local-tools/v1".to_owned()),
            reasoning: model.reasoning(),
            reasoning_label,
            badges,
            enabled: model.is_enabled(),
            disabled_reason,
        }
    }

    #[cfg(test)]
    pub(crate) fn model_id(&self) -> &str {
        &self.model_id
    }

    #[cfg(test)]
    pub(crate) const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[cfg(test)]
    pub(crate) fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }
}

fn generic_reasoning_label(reasoning: Option<bool>) -> Option<String> {
    reasoning.map(|available| {
        if available {
            "reasoning available/off"
        } else {
            "reasoning unavailable/off"
        }
        .to_owned()
    })
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
    reasoning_label: Option<String>,
    badges: Vec<String>,
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
            reasoning_label: model.reasoning_label.clone(),
            badges: model.badges.clone(),
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
        if let Some((start, _)) = self.query.grapheme_indices(true).next_back() {
            self.query.truncate(start);
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
    let mut lines = vec![styled(
        &clip_line("Model catalog", width),
        TextStyle::Accent,
        style,
    )];
    lines.extend(wrap_text(
        &format!("Provider  {}", escape_remote_text(&identity.provider)),
        width,
    ));
    lines.extend(wrap_text(
        &format!("Account  {}", escape_remote_text(&identity.account)),
        width,
    ));
    lines.extend([
        styled(
            &clip_line("Select one model", width),
            TextStyle::Bold,
            style,
        ),
        clip_line(
            &format!("Search  {}_", escape_remote_text(&state.query)),
            width,
        ),
        String::new(),
    ]);
    if state.matches.is_empty() {
        lines.push(styled(
            &clip_line("  No matching models", width),
            TextStyle::Muted,
            style,
        ));
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
            let reasoning = choice
                .reasoning_label
                .as_deref()
                .unwrap_or(match choice.reasoning {
                    Some(true) => "reasoning",
                    Some(false) => "no reasoning",
                    None => "reasoning ?",
                });
            let badges = if choice.badges.is_empty() {
                String::new()
            } else {
                format!(" · {}", choice.badges.join(" · "))
            };
            let availability = if choice.enabled { "ready" } else { "disabled" };
            let row = format!(
                "{marker} {}  {} ctx · {} out · {tools} · {reasoning}{badges} · {availability}",
                escape_remote_text(&choice.display_name),
                readable_optional_limit(choice.input_limit),
                readable_optional_limit(choice.output_limit),
            );
            lines.push(if state.selected == Some(match_index) {
                styled(&clip_line(&row, width), TextStyle::Selected, style)
            } else {
                clip_line(&row, width)
            });
        }
    }
    lines.push(String::new());
    if let Some(index) = state.selected_model_index() {
        let id = format!("Model  {}", escape_remote_text(&choices[index].model_id));
        lines.extend(wrap_text(&id, width));
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
        lines.extend(wrap_text(&availability, width));
    }
    lines.push(styled(
        &clip_line(
            "↑↓ navigate · type to filter · Enter select · Esc cancel",
            width,
        ),
        TextStyle::Muted,
        style,
    ));
    lines
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

fn styled(value: &str, text_style: TextStyle, style: PresentationStyle) -> String {
    style.decorate(text_style, value)
}

fn clip_line(value: &str, width: usize) -> String {
    if text_width(value) <= width {
        return value.to_owned();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let content_width = width - 3;
    let mut clipped = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = terminal_grapheme_width(grapheme);
        if used + grapheme_width > content_width {
            break;
        }
        clipped.push_str(grapheme);
        used += grapheme_width;
    }
    clipped.push_str("...");
    clipped
}

fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = terminal_grapheme_width(grapheme);
        if !line.is_empty() && used + grapheme_width > width {
            lines.push(line);
            line = String::new();
            used = 0;
        }
        if grapheme_width > width {
            continue;
        }
        line.push_str(grapheme);
        used += grapheme_width;
    }
    if !line.is_empty() || value.is_empty() {
        lines.push(line);
    }
    lines
}

fn text_width(value: &str) -> usize {
    value.graphemes(true).map(terminal_grapheme_width).sum()
}

fn terminal_grapheme_width(value: &str) -> usize {
    usize::from(
        Grapheme::try_from(value)
            .expect("picker text must contain terminal-safe graphemes")
            .width()
            .get(),
    )
}

#[cfg(test)]
mod tests;
