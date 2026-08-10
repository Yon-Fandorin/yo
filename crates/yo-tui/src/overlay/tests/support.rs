use super::super::{PanelSnapshot, SelectionEntry};

pub(super) fn enabled(id: &str, label: &str) -> SelectionEntry {
    SelectionEntry::enabled(id, label, None)
}

pub(super) fn snapshot(entries: Vec<SelectionEntry>) -> PanelSnapshot {
    PanelSnapshot::new("Commands", entries).unwrap()
}
