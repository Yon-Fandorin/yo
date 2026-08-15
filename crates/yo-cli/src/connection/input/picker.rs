use std::fs::File;

use yo_core::OpenRouterDiscoveredModel;

use super::super::presentation::{PresentationStyle, default_width, escape_remote_text};
use crate::AppError;

const MAX_VISIBLE_RESULTS: usize = 8;
const MAX_QUERY_BYTES: usize = 1024;

mod terminal;

use self::terminal::{PickerKey, PickerTerminalScope, read_key};

pub(super) fn select_model(
    terminal: &mut File,
    models: &[OpenRouterDiscoveredModel],
    style: PresentationStyle,
) -> Result<Option<usize>, AppError> {
    if models.is_empty() {
        return Err(AppError::message(
            "OpenRouter discovery returned no selectable text-and-tools model",
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
                if let Some(selected) = state.selected_model_index() {
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

#[derive(Debug)]
struct PickerIdentity {
    provider: String,
    account: String,
}

impl PickerIdentity {
    fn from_models(models: &[OpenRouterDiscoveredModel]) -> Result<Self, AppError> {
        let first = models[0].entry().binding();
        if models.iter().any(|model| {
            let binding = model.entry().binding();
            binding.provider_id() != first.provider_id()
                || binding.account_id() != first.account_id()
        }) {
            return Err(AppError::message(
                "OpenRouter picker models must belong to one Provider and Account",
            ));
        }
        Ok(Self {
            provider: first.provider_id().as_str().to_owned(),
            account: first.account_id().as_str().to_owned(),
        })
    }
}

#[derive(Clone, Debug)]
struct PickerChoice {
    display_name: String,
    model_id: String,
    input_limit: u64,
    output_limit: u64,
    reasoning: bool,
}

impl From<&OpenRouterDiscoveredModel> for PickerChoice {
    fn from(model: &OpenRouterDiscoveredModel) -> Self {
        let complete = model
            .entry()
            .complete_binding()
            .expect("discovered models always carry complete profiles");
        Self {
            display_name: model.display_name().to_owned(),
            model_id: complete.binding().model_id().as_str().to_owned(),
            input_limit: complete.profile().context().input_token_limit(),
            output_limit: complete.profile().context().max_output_tokens(),
            reasoning: model.reasoning(),
        }
    }
}

#[derive(Debug)]
struct PickerState {
    query: String,
    matches: Vec<usize>,
    selected: Option<usize>,
    viewport_start: usize,
}

impl PickerState {
    fn new(choices: &[PickerChoice]) -> Self {
        let mut state = Self {
            query: String::new(),
            matches: Vec::new(),
            selected: None,
            viewport_start: 0,
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
        if selected >= self.viewport_start + MAX_VISIBLE_RESULTS {
            self.viewport_start = selected + 1 - MAX_VISIBLE_RESULTS;
        }
    }

    fn selected_model_index(&self) -> Option<usize> {
        self.selected.map(|selected| self.matches[selected])
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
    let mut lines = vec![styled("Model discovery", "\x1b[1;36m", style)];
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
            let badge = if choice.reasoning {
                " · reasoning"
            } else {
                ""
            };
            let row = format!(
                "{marker} {}  {} ctx · {} out · tools{badge}",
                escape_remote_text(&choice.display_name),
                readable_limit(choice.input_limit),
                readable_limit(choice.output_limit),
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
