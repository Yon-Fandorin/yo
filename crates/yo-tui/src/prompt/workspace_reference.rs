//! Pure `@` trigger scanning and revision-bound selection state.

use std::{collections::HashMap, ops::Range};

use unicode_segmentation::UnicodeSegmentation;
use yo_core::{
    WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchStatus, WorkspaceReferenceSearchUpdate,
};

use crate::{
    input::editor::PromptEditor,
    overlay::{
        AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, PromptOverlaySlot, SelectionEntry,
    },
    surface::Grapheme,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AssistTrigger {
    pub(super) kind: AssistTriggerKind,
    pub(super) span: Range<usize>,
    pub(super) query: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AssistTriggerKind {
    Workspace,
    Skill,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceEdit {
    pub(super) old: Range<usize>,
    pub(super) new: Range<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct WorkspaceReferenceAssist {
    active: Option<ActiveSearch>,
    accepted: Vec<AcceptedAnnotation>,
    last_text: String,
    #[cfg(test)]
    test_editor_revision: u64,
    #[cfg(test)]
    test_next_request_id: u64,
}

#[derive(Debug)]
struct ActiveSearch {
    trigger: AssistTrigger,
    request_id: u64,
    token: OverlayInstanceToken,
    sequence: Option<u64>,
    terminal: bool,
    expected_trigger: String,
    candidates: HashMap<String, WorkspaceReferenceCandidate>,
}

#[derive(Debug)]
struct AcceptedAnnotation {
    span: Range<usize>,
    projection: String,
    reference: WorkspaceReference,
}

impl WorkspaceReferenceAssist {
    #[cfg(test)]
    pub(crate) const fn enable(&mut self) {}

    #[cfg(test)]
    pub(crate) fn prompt_changed(
        &mut self,
        editor: &PromptEditor,
        overlay: &mut PromptOverlaySlot,
        edit: Option<&WorkspaceEdit>,
        eligible: bool,
    ) -> Option<(WorkspaceReferenceSearchRequest, bool)> {
        self.update_annotations(editor.text(), edit);
        self.test_editor_revision = self.test_editor_revision.saturating_add(1);
        if !eligible {
            self.close(overlay);
            return None;
        }
        let trigger = scan_prompt_trigger(editor.text(), editor.cursor_byte_index())?;
        if trigger.kind != AssistTriggerKind::Workspace {
            self.close(overlay);
            return None;
        }
        self.test_next_request_id = self.test_next_request_id.saturating_add(1);
        self.begin(
            editor,
            overlay,
            trigger,
            self.test_next_request_id,
            self.test_editor_revision,
        )
    }

    pub(super) fn begin(
        &mut self,
        editor: &PromptEditor,
        overlay: &mut PromptOverlaySlot,
        trigger: AssistTrigger,
        request_id: u64,
        editor_revision: u64,
    ) -> Option<(WorkspaceReferenceSearchRequest, bool)> {
        if self.accepted.iter().any(|annotation| {
            ranges_intersect(&annotation.span, &trigger.span)
                || editor.cursor_byte_index() == annotation.span.end
        }) {
            self.close(overlay);
            return None;
        }
        let expected_trigger = editor.text()[trigger.span.clone()].to_owned();
        let (token, show_loading) = if let Some(active) = self.active.take() {
            (active.token, false)
        } else {
            let snapshot = status_snapshot("Files", "Preparing results…")
                .with_title_status("Searching…")
                .ok()?;
            (overlay.open(snapshot).ok()?, true)
        };
        overlay.set_acceptance_enabled(token, false).ok()?;
        let request = WorkspaceReferenceSearchRequest::new(
            request_id,
            editor_revision,
            editor.cursor_byte_index(),
            trigger.span.clone(),
            expected_trigger.clone(),
            trigger.query.clone(),
        );
        self.active = Some(ActiveSearch {
            trigger,
            request_id,
            token,
            sequence: None,
            terminal: false,
            expected_trigger,
            candidates: HashMap::new(),
        });
        Some((request, show_loading))
    }

    pub(crate) fn observe(
        &mut self,
        update: WorkspaceReferenceSearchUpdate,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.request_id != update.request_id()
            || active.terminal
            || active
                .sequence
                .is_some_and(|sequence| update.sequence() <= sequence)
        {
            return false;
        }
        active.sequence = Some(update.sequence());
        active.terminal = update.is_final();
        active.candidates.clear();
        if let WorkspaceReferenceSearchStatus::Failed(reason) = update.status() {
            let refreshed = overlay
                .refresh(active.token, status_snapshot("Files", reason))
                .is_ok();
            if refreshed {
                let _ = overlay.set_acceptance_enabled(active.token, false);
            }
            return refreshed;
        }
        let mut entries = Vec::new();
        for candidate in update.candidates() {
            let identity = candidate.reference().identity().to_owned();
            active
                .candidates
                .insert(identity.clone(), candidate.clone());
            let context = (!candidate.detail().is_empty())
                .then(|| display_candidate_text(candidate.detail()));
            let kind = match candidate.reference().kind() {
                yo_core::WorkspaceReferenceKind::File => "File",
                yo_core::WorkspaceReferenceKind::Directory => "Dir",
            };
            entries.push(SelectionEntry::enabled_with_context(
                identity,
                display_candidate_text(candidate.label()),
                context,
                Some(kind.to_owned()),
            ));
        }
        if entries.is_empty() {
            let message = match update.status() {
                WorkspaceReferenceSearchStatus::Complete => {
                    "No matching workspace paths".to_owned()
                },
                WorkspaceReferenceSearchStatus::Incomplete(reason)
                | WorkspaceReferenceSearchStatus::Failed(reason) => reason.clone(),
            };
            entries.push(SelectionEntry::disabled(
                "status",
                message,
                None,
                "not selectable",
            ));
        } else if let WorkspaceReferenceSearchStatus::Incomplete(reason)
        | WorkspaceReferenceSearchStatus::Failed(reason) = update.status()
        {
            entries.push(SelectionEntry::disabled(
                "provider-status",
                "Results may be incomplete",
                Some(reason.clone()),
                "not selectable",
            ));
        }
        let snapshot = PanelSnapshot::new("Files", entries).unwrap_or_else(|_| {
            active.candidates.clear();
            status_snapshot("Files", "Workspace results cannot be displayed safely")
        });
        let selectable = !active.candidates.is_empty();
        let refreshed = overlay.refresh(active.token, snapshot).is_ok();
        if refreshed {
            let _ = overlay.set_acceptance_enabled(active.token, selectable);
        }
        refreshed
    }

    pub(crate) fn provider_failed(
        &mut self,
        reason: String,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.terminal {
            return false;
        }
        active.terminal = true;
        active.candidates.clear();
        let refreshed = overlay
            .refresh(active.token, status_snapshot("Files", &reason))
            .is_ok();
        if refreshed {
            let _ = overlay.set_acceptance_enabled(active.token, false);
        }
        refreshed
    }

    pub(crate) fn accept(
        &mut self,
        receipt: &AcceptanceReceipt,
        editor: &mut PromptEditor,
    ) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        if receipt.token() != active.token {
            self.active = Some(active);
            return false;
        }
        let Some(candidate) = active.candidates.get(receipt.identity()) else {
            return false;
        };
        if editor
            .text()
            .get(active.trigger.span.clone())
            .is_none_or(|text| text != active.expected_trigger)
        {
            return false;
        }
        let suffix = match candidate.reference().kind() {
            yo_core::WorkspaceReferenceKind::Directory => "/",
            yo_core::WorkspaceReferenceKind::File => "",
        };
        let visible_path = display_candidate_text(candidate.reference().relative_path());
        let replacement = format!("@{visible_path}{suffix}");
        let start = active.trigger.span.start;
        editor.replace_range(active.trigger.span, &replacement);
        self.accepted.push(AcceptedAnnotation {
            span: start..start + replacement.len(),
            projection: replacement,
            reference: candidate.reference().clone(),
        });
        self.last_text = editor.text().to_owned();
        true
    }

    pub(crate) fn has_accepted_references(&self) -> bool {
        !self.accepted.is_empty()
    }

    pub(crate) fn cancel(&mut self) {
        self.active = None;
    }

    pub(super) fn update_annotations(&mut self, text: &str, edit: Option<&WorkspaceEdit>) {
        self.transform_annotations(text, edit);
    }

    pub(super) fn close(&mut self, overlay: &mut PromptOverlaySlot) {
        if let Some(active) = self.active.take() {
            let _ = overlay.close(active.token);
        }
    }

    fn transform_annotations(&mut self, text: &str, edit: Option<&WorkspaceEdit>) {
        let (old_change, new_change) = edit
            .map(|edit| (edit.old.clone(), edit.new.clone()))
            .unwrap_or_else(|| changed_ranges(&self.last_text, text));
        let delta = isize::try_from(new_change.len()).unwrap_or(isize::MAX)
            - isize::try_from(old_change.len()).unwrap_or(isize::MAX);
        self.accepted.retain_mut(|annotation| {
            if old_change.end <= annotation.span.start {
                annotation.span = shift_range(annotation.span.clone(), delta);
            } else if old_change.start < annotation.span.end {
                return false;
            }
            text.get(annotation.span.clone())
                .is_some_and(|projection| projection == annotation.projection)
                && !annotation.reference.identity().is_empty()
        });
        self.last_text = text.to_owned();
    }
}

impl WorkspaceEdit {
    pub(crate) fn between(
        old: &str,
        old_cursor: usize,
        new: &str,
        new_cursor: usize,
    ) -> Option<Self> {
        if old == new {
            return None;
        }
        if new.len() > old.len()
            && new_cursor >= old_cursor
            && new.get(..old_cursor) == old.get(..old_cursor)
            && new.get(new_cursor..) == old.get(old_cursor..)
        {
            return Some(Self {
                old: old_cursor..old_cursor,
                new: old_cursor..new_cursor,
            });
        }
        if new.len() < old.len()
            && new_cursor <= old_cursor
            && new.get(..new_cursor) == old.get(..new_cursor)
        {
            let removed = old.len() - new.len();
            let old_end = if new_cursor < old_cursor {
                old_cursor
            } else {
                old_cursor.saturating_add(removed)
            };
            if old.get(old_end..) == new.get(new_cursor..) {
                return Some(Self {
                    old: new_cursor..old_end,
                    new: new_cursor..new_cursor,
                });
            }
        }
        let (old, new) = changed_ranges(old, new);
        Some(Self { old, new })
    }
}

fn changed_ranges(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let prefix = old
        .char_indices()
        .zip(new.char_indices())
        .take_while(|((_, left), (_, right))| left == right)
        .map(|((index, character), _)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    let mut suffix = 0;
    for (left, right) in old[prefix..].chars().rev().zip(new[prefix..].chars().rev()) {
        if left != right {
            break;
        }
        suffix += left.len_utf8();
    }
    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}

fn shift_range(range: Range<usize>, delta: isize) -> Range<usize> {
    range.start.saturating_add_signed(delta)..range.end.saturating_add_signed(delta)
}

fn ranges_intersect(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn status_snapshot(title: &str, message: &str) -> PanelSnapshot {
    PanelSnapshot::new(
        title,
        vec![SelectionEntry::disabled(
            "status",
            display_candidate_text(message),
            None,
            "not selectable",
        )],
    )
    .expect("built-in workspace status rows are valid")
}

pub(super) fn display_candidate_text(text: &str) -> String {
    text.graphemes(true)
        .map(|cluster| {
            if Grapheme::try_from(cluster).is_ok() {
                cluster.to_owned()
            } else {
                cluster
                    .chars()
                    .map(|character| format!("\\u{{{:X}}}", u32::from(character)))
                    .collect()
            }
        })
        .collect()
}

pub(crate) fn scan_prompt_trigger(text: &str, cursor: usize) -> Option<AssistTrigger> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let token_start = text[..cursor]
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            character
                .is_whitespace()
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let token_end = text[cursor..]
        .char_indices()
        .find_map(|(offset, character)| character.is_whitespace().then_some(cursor + offset))
        .unwrap_or(text.len());
    let token = &text[token_start..token_end];
    let (kind, query) = token
        .strip_prefix('@')
        .map(|query| (AssistTriggerKind::Workspace, query))
        .or_else(|| {
            token
                .strip_prefix('$')
                .map(|query| (AssistTriggerKind::Skill, query))
        })?;
    if token_start > 0
        && !text[..token_start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        return None;
    }
    Some(AssistTrigger {
        kind,
        span: token_start..token_end,
        query: query.to_owned(),
    })
}

#[cfg(test)]
mod tests;
