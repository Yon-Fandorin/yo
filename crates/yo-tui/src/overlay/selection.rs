use std::collections::HashSet;

use unicode_segmentation::UnicodeSegmentation;

use crate::surface::{Grapheme, GraphemeError, Style};

const VISIBLE_ENTRY_CAP: usize = 8;

mod render;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EntryIdentity(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EntryAvailability {
    Enabled,
    Disabled { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectionEntry {
    identity: EntryIdentity,
    label: String,
    context: Option<String>,
    detail: Option<String>,
    availability: EntryAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PanelSnapshot {
    title: String,
    title_status: Option<PanelTitleStatus>,
    entries: Vec<SelectionEntry>,
    filter_bar: Option<FilterBar>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum PanelTitleStatus {
    Static(String),
    Activity(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FilterBar {
    labels: Vec<String>,
    selected: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectionPanel {
    snapshot: PanelSnapshot,
    selected: Option<EntryIdentity>,
    freshness: SnapshotFreshness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotFreshness {
    Fresh,
    PendingReplacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectionPanelStyles {
    pub(crate) background: Style,
    pub(crate) frame: Style,
    pub(crate) title: Style,
    pub(crate) key_hint: Style,
    pub(crate) hint: Style,
    pub(crate) label: Style,
    pub(crate) detail: Style,
    pub(crate) selected: Style,
    pub(crate) disabled: Style,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectionPanelGlyphs {
    horizontal: &'static str,
    vertical: &'static str,
    top_left: &'static str,
    top_right: &'static str,
    bottom_left: &'static str,
    bottom_right: &'static str,
    selected_marker: &'static str,
    rich_keys: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectionPanelAppearance {
    pub(crate) styles: SelectionPanelStyles,
    pub(crate) glyphs: SelectionPanelGlyphs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavigationOutcome {
    SelectionChanged,
    HandledNoSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelValidationError {
    EmptyTitle,
    EmptyEntries,
    EmptyIdentity {
        index: usize,
    },
    DuplicateIdentity {
        index: usize,
    },
    EmptyLabel {
        index: usize,
    },
    EmptyDisabledReason {
        index: usize,
    },
    EmptyFilterLabels,
    FilterSelectionOutOfRange,
    UnsafeText {
        field: TextField,
        cause: GraphemeError,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextField {
    Title,
    TitleStatus,
    Label { index: usize },
    Context { index: usize },
    Detail { index: usize },
    DisabledReason { index: usize },
    FilterLabel { index: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelPaintError {
    SurfaceConflict,
}

impl EntryIdentity {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl SelectionEntry {
    #[cfg(test)]
    pub(crate) fn enabled(
        identity: impl Into<String>,
        label: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            identity: EntryIdentity::new(identity),
            label: label.into(),
            context: None,
            detail,
            availability: EntryAvailability::Enabled,
        }
    }

    pub(crate) fn enabled_with_context(
        identity: impl Into<String>,
        label: impl Into<String>,
        context: Option<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            identity: EntryIdentity::new(identity),
            label: label.into(),
            context,
            detail,
            availability: EntryAvailability::Enabled,
        }
    }

    pub(crate) fn disabled(
        identity: impl Into<String>,
        label: impl Into<String>,
        detail: Option<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            identity: EntryIdentity::new(identity),
            label: label.into(),
            context: None,
            detail,
            availability: EntryAvailability::Disabled {
                reason: reason.into(),
            },
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(self.availability, EntryAvailability::Enabled)
    }
}

impl PanelSnapshot {
    pub(crate) fn new(
        title: impl Into<String>,
        entries: Vec<SelectionEntry>,
    ) -> Result<Self, PanelValidationError> {
        let snapshot = Self {
            title: title.into(),
            title_status: None,
            entries,
            filter_bar: None,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_title_status(
        mut self,
        status: impl Into<String>,
    ) -> Result<Self, PanelValidationError> {
        self.title_status = Some(PanelTitleStatus::Static(status.into()));
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn with_activity_title_status(
        mut self,
        status: impl Into<String>,
    ) -> Result<Self, PanelValidationError> {
        self.title_status = Some(PanelTitleStatus::Activity(status.into()));
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn with_filter_bar(
        mut self,
        labels: impl IntoIterator<Item = impl Into<String>>,
        selected: usize,
    ) -> Result<Self, PanelValidationError> {
        self.filter_bar = Some(FilterBar {
            labels: labels.into_iter().map(Into::into).collect(),
            selected,
        });
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), PanelValidationError> {
        if self.title.is_empty() {
            return Err(PanelValidationError::EmptyTitle);
        }
        validate_text(&self.title, TextField::Title)?;
        if let Some(status) = &self.title_status {
            validate_text(status.text(), TextField::TitleStatus)?;
        }
        if self.entries.is_empty() {
            return Err(PanelValidationError::EmptyEntries);
        }
        if let Some(filter_bar) = &self.filter_bar {
            if filter_bar.labels.is_empty() {
                return Err(PanelValidationError::EmptyFilterLabels);
            }
            if filter_bar.selected >= filter_bar.labels.len() {
                return Err(PanelValidationError::FilterSelectionOutOfRange);
            }
            for (index, label) in filter_bar.labels.iter().enumerate() {
                if label.is_empty() {
                    return Err(PanelValidationError::EmptyFilterLabels);
                }
                validate_text(label, TextField::FilterLabel { index })?;
            }
        }
        let mut identities = HashSet::with_capacity(self.entries.len());
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.identity.0.is_empty() {
                return Err(PanelValidationError::EmptyIdentity { index });
            }
            if !identities.insert(entry.identity.clone()) {
                return Err(PanelValidationError::DuplicateIdentity { index });
            }
            if entry.label.is_empty() {
                return Err(PanelValidationError::EmptyLabel { index });
            }
            validate_text(&entry.label, TextField::Label { index })?;
            if let Some(context) = &entry.context {
                validate_text(context, TextField::Context { index })?;
            }
            if let Some(detail) = &entry.detail {
                validate_text(detail, TextField::Detail { index })?;
            }
            if let EntryAvailability::Disabled { reason } = &entry.availability {
                if reason.is_empty() {
                    return Err(PanelValidationError::EmptyDisabledReason { index });
                }
                validate_text(reason, TextField::DisabledReason { index })?;
            }
        }
        Ok(())
    }
}

impl SelectionPanel {
    pub(crate) fn new(snapshot: PanelSnapshot) -> Self {
        let selected = snapshot
            .entries
            .iter()
            .find(|entry| entry.is_enabled())
            .map(|entry| entry.identity.clone());
        Self {
            snapshot,
            selected,
            freshness: SnapshotFreshness::Fresh,
        }
    }

    pub(crate) fn refresh(&mut self, snapshot: PanelSnapshot) {
        let selected = self.selected.as_ref().and_then(|selected| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.identity == *selected && entry.is_enabled())
                .map(|entry| entry.identity.clone())
        });
        self.selected = selected.or_else(|| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.is_enabled())
                .map(|entry| entry.identity.clone())
        });
        self.snapshot = snapshot;
        self.freshness = SnapshotFreshness::Fresh;
    }

    pub(crate) fn set_pending_activity(
        &mut self,
        status: impl Into<String>,
    ) -> Result<(), PanelValidationError> {
        let status = PanelTitleStatus::Activity(status.into());
        validate_text(status.text(), TextField::TitleStatus)?;
        self.snapshot.title_status = Some(status);
        self.freshness = SnapshotFreshness::PendingReplacement;
        Ok(())
    }

    pub(crate) fn previous(&mut self) -> NavigationOutcome {
        self.move_selection(Direction::Previous)
    }

    pub(crate) fn next(&mut self) -> NavigationOutcome {
        self.move_selection(Direction::Next)
    }

    pub(crate) fn selected_identity(&self) -> Option<&EntryIdentity> {
        self.selected.as_ref()
    }

    pub(crate) fn is_fresh(&self) -> bool {
        self.freshness == SnapshotFreshness::Fresh
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[SelectionEntry] {
        &self.snapshot.entries
    }

    #[cfg(test)]
    pub(crate) fn has_activity_title_status(&self) -> bool {
        matches!(
            self.snapshot.title_status,
            Some(PanelTitleStatus::Activity(_))
        )
    }

    pub(crate) fn previous_filter(&mut self) -> Option<usize> {
        let filter = self.snapshot.filter_bar.as_mut()?;
        filter.selected = if filter.selected == 0 {
            filter.labels.len() - 1
        } else {
            filter.selected - 1
        };
        Some(filter.selected)
    }

    pub(crate) fn next_filter(&mut self) -> Option<usize> {
        let filter = self.snapshot.filter_bar.as_mut()?;
        filter.selected = (filter.selected + 1) % filter.labels.len();
        Some(filter.selected)
    }

    pub(crate) const fn has_filter_bar(&self) -> bool {
        self.snapshot.filter_bar.is_some()
    }

    fn move_selection(&mut self, direction: Direction) -> NavigationOutcome {
        let enabled = self
            .snapshot
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.is_enabled())
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            self.selected = None;
            return NavigationOutcome::HandledNoSelection;
        }
        let current = self.selected.as_ref().and_then(|selected| {
            enabled
                .iter()
                .position(|index| self.snapshot.entries[*index].identity == *selected)
        });
        let position = match (direction, current) {
            (Direction::Next, Some(position)) => (position + 1) % enabled.len(),
            (Direction::Previous, Some(0)) => enabled.len() - 1,
            (Direction::Previous, Some(position)) => position - 1,
            (_, None) => 0,
        };
        self.selected = Some(self.snapshot.entries[enabled[position]].identity.clone());
        NavigationOutcome::SelectionChanged
    }

    fn visible_window(&self, visible_rows: usize) -> (usize, usize) {
        let selected = self.selected.as_ref().and_then(|identity| {
            self.snapshot
                .entries
                .iter()
                .position(|entry| entry.identity == *identity)
        });
        let start = selected
            .map(|index| index.saturating_sub(visible_rows - 1))
            .unwrap_or(0)
            .min(self.snapshot.entries.len() - visible_rows);
        (start, start + visible_rows)
    }
}

impl PanelTitleStatus {
    pub(super) fn text(&self) -> &str {
        match self {
            Self::Static(text) | Self::Activity(text) => text,
        }
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Previous,
    Next,
}

fn validate_text(value: &str, field: TextField) -> Result<(), PanelValidationError> {
    for cluster in value.graphemes(true) {
        Grapheme::try_from(cluster)
            .map_err(|cause| PanelValidationError::UnsafeText { field, cause })?;
    }
    Ok(())
}

impl SelectionPanelGlyphs {
    pub(crate) const fn rich() -> Self {
        Self {
            horizontal: "─",
            vertical: "│",
            top_left: "╭",
            top_right: "╮",
            bottom_left: "╰",
            bottom_right: "╯",
            selected_marker: "›",
            rich_keys: true,
        }
    }

    pub(crate) const fn ascii() -> Self {
        Self {
            horizontal: "-",
            vertical: "|",
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            selected_marker: ">",
            rich_keys: false,
        }
    }
}
