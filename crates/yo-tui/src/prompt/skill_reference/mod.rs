//! `$` skill catalog assist with typed selection and client-side provenance filters.

use std::{collections::HashMap, ops::Range};

use yo_core::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceScope,
    SkillReferenceSearchRequest, SkillReferenceSearchStatus, SkillReferenceSearchUpdate,
};

#[cfg(test)]
use crate::prompt::workspace_reference::{AssistTriggerKind, scan_prompt_trigger};
use crate::{
    input::editor::PromptEditor,
    overlay::{
        AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, PromptOverlaySlot, SelectionEntry,
    },
    prompt::workspace_reference::{AssistTrigger, WorkspaceEdit, display_candidate_text},
};

const FILTER_LABELS: [&str; 5] = ["All", "Workspace", "User", "System", "Admin"];

#[derive(Debug, Default)]
pub(crate) struct SkillReferenceAssist {
    active: Option<ActiveSearch>,
    accepted: Option<AcceptedAnnotation>,
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
    candidates: Vec<SkillReferenceCandidate>,
    visible: HashMap<String, SkillReferenceCandidate>,
    status: SkillReferenceSearchStatus,
    filter: SkillFilter,
}

#[derive(Debug)]
struct AcceptedAnnotation {
    span: Range<usize>,
    projection: String,
    reference: SkillReference,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SkillFilter {
    #[default]
    All,
    Workspace,
    User,
    System,
    Admin,
}

impl SkillReferenceAssist {
    #[cfg(test)]
    pub(crate) const fn enable(&mut self) {}

    #[cfg(test)]
    pub(crate) fn prompt_changed(
        &mut self,
        editor: &PromptEditor,
        overlay: &mut PromptOverlaySlot,
        edit: Option<&WorkspaceEdit>,
        eligible: bool,
    ) -> Option<(SkillReferenceSearchRequest, bool)> {
        self.update_annotation(editor.text(), edit);
        self.test_editor_revision = self.test_editor_revision.saturating_add(1);
        if !eligible {
            self.close(overlay);
            return None;
        }
        let trigger = scan_prompt_trigger(editor.text(), editor.cursor_byte_index())?;
        if trigger.kind != AssistTriggerKind::Skill {
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
    ) -> Option<(SkillReferenceSearchRequest, bool)> {
        if self.accepted.is_some() {
            self.close(overlay);
            let snapshot = filtered_status_snapshot(
                "Version 1 supports one explicit skill per request",
                SkillFilter::All,
            );
            let token = overlay.open(snapshot).ok()?;
            overlay.set_acceptance_enabled(token, false).ok()?;
            self.active = Some(ActiveSearch {
                expected_trigger: editor.text()[trigger.span.clone()].to_owned(),
                trigger,
                request_id,
                token,
                sequence: None,
                terminal: true,
                candidates: Vec::new(),
                visible: HashMap::new(),
                status: SkillReferenceSearchStatus::Failed(
                    "Version 1 supports one explicit skill per request".to_owned(),
                ),
                filter: SkillFilter::All,
            });
            return None;
        }
        let expected_trigger = editor.text()[trigger.span.clone()].to_owned();
        let (token, show_loading) = if let Some(active) = self.active.take() {
            (active.token, false)
        } else {
            let snapshot = filtered_status_snapshot("Preparing results…", SkillFilter::All)
                .with_title_status("Searching…")
                .ok()?;
            (overlay.open(snapshot).ok()?, true)
        };
        overlay.set_acceptance_enabled(token, false).ok()?;
        let request = SkillReferenceSearchRequest::new(
            request_id,
            editor_revision,
            editor.cursor_byte_index(),
            trigger.span.clone(),
            expected_trigger.clone(),
            trigger.query.clone(),
            show_loading,
        );
        self.active = Some(ActiveSearch {
            trigger,
            request_id,
            token,
            sequence: None,
            terminal: false,
            expected_trigger,
            candidates: Vec::new(),
            visible: HashMap::new(),
            status: SkillReferenceSearchStatus::Complete,
            filter: SkillFilter::All,
        });
        Some((request, show_loading))
    }

    pub(crate) fn observe(
        &mut self,
        update: SkillReferenceSearchUpdate,
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
        active.status = update.status().clone();
        active.candidates = if matches!(update.status(), SkillReferenceSearchStatus::Failed(_)) {
            Vec::new()
        } else {
            update.candidates().to_vec()
        };
        refresh_active(active, overlay)
    }

    pub(crate) fn filter_changed(
        &mut self,
        selected: usize,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let Some(filter) = SkillFilter::from_index(selected) else {
            return false;
        };
        active.filter = filter;
        refresh_active(active, overlay)
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
        active.status = SkillReferenceSearchStatus::Failed(reason);
        active.candidates.clear();
        refresh_active(active, overlay)
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
        let Some(candidate) = active.visible.get(receipt.identity()) else {
            return false;
        };
        if !matches!(candidate.availability(), SkillAvailability::Enabled)
            || editor
                .text()
                .get(active.trigger.span.clone())
                .is_none_or(|text| text != active.expected_trigger)
        {
            return false;
        }
        let replacement = yo_core::skill_reference_projection(candidate.reference());
        let start = active.trigger.span.start;
        editor.replace_range(active.trigger.span, &replacement);
        self.accepted = Some(AcceptedAnnotation {
            span: start..start + replacement.len(),
            projection: replacement,
            reference: candidate.reference().clone(),
        });
        self.last_text = editor.text().to_owned();
        true
    }

    pub(crate) const fn has_accepted_reference(&self) -> bool {
        self.accepted.is_some()
    }

    pub(super) fn trigger_is_accepted(&self, trigger: &AssistTrigger) -> bool {
        self.accepted
            .as_ref()
            .is_some_and(|annotation| ranges_intersect(&annotation.span, &trigger.span))
    }

    #[cfg(test)]
    pub(crate) fn accepted_reference(&self) -> Option<&SkillReference> {
        self.accepted
            .as_ref()
            .map(|annotation| &annotation.reference)
    }

    pub(crate) fn cancel(&mut self) {
        self.active = None;
    }

    pub(super) fn update_annotation(&mut self, text: &str, edit: Option<&WorkspaceEdit>) {
        self.transform_annotation(text, edit);
    }

    pub(super) fn close(&mut self, overlay: &mut PromptOverlaySlot) {
        if let Some(active) = self.active.take() {
            let _ = overlay.close(active.token);
        }
    }

    fn transform_annotation(&mut self, text: &str, edit: Option<&WorkspaceEdit>) {
        let Some(annotation) = self.accepted.as_mut() else {
            self.last_text = text.to_owned();
            return;
        };
        let Some(edit) = edit else {
            if text != self.last_text {
                self.accepted = None;
            }
            self.last_text = text.to_owned();
            return;
        };
        if edit.old.end <= annotation.span.start {
            let delta = isize::try_from(edit.new.len()).unwrap_or(isize::MAX)
                - isize::try_from(edit.old.len()).unwrap_or(isize::MAX);
            annotation.span = annotation.span.start.saturating_add_signed(delta)
                ..annotation.span.end.saturating_add_signed(delta);
        } else if edit.old.start < annotation.span.end {
            self.accepted = None;
        }
        if self.accepted.as_ref().is_some_and(|annotation| {
            text.get(annotation.span.clone()) != Some(annotation.projection.as_str())
                || annotation.reference.identity().is_empty()
        }) {
            self.accepted = None;
        }
        self.last_text = text.to_owned();
    }
}

fn ranges_intersect(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn refresh_active(active: &mut ActiveSearch, overlay: &mut PromptOverlaySlot) -> bool {
    active.visible.clear();
    let mut entries = Vec::new();
    for candidate in active
        .candidates
        .iter()
        .filter(|candidate| active.filter.matches(candidate.reference().scope()))
    {
        let identity = candidate.reference().identity().to_owned();
        active.visible.insert(identity.clone(), candidate.clone());
        let scope = scope_label(candidate.reference().scope()).to_owned();
        let label = display_candidate_text(candidate.display_name());
        let description = Some(display_candidate_text(candidate.description()));
        entries.push(match candidate.availability() {
            SkillAvailability::Enabled => {
                SelectionEntry::enabled_with_context(identity, label, description, Some(scope))
            },
            SkillAvailability::Disabled(reason) => SelectionEntry::disabled(
                identity,
                label,
                Some(scope),
                display_candidate_text(reason),
            ),
        });
    }
    if entries.is_empty() {
        entries.push(SelectionEntry::disabled(
            "status",
            status_message(&active.status, active.filter),
            None,
            "not selectable",
        ));
    } else if let SkillReferenceSearchStatus::Incomplete(reason)
    | SkillReferenceSearchStatus::Failed(reason) = &active.status
    {
        entries.push(SelectionEntry::disabled(
            "provider-status",
            "Results may be incomplete",
            Some(display_candidate_text(reason)),
            "not selectable",
        ));
    }
    let snapshot = PanelSnapshot::new("Skills", entries)
        .and_then(|snapshot| snapshot.with_filter_bar(FILTER_LABELS, active.filter.index()))
        .unwrap_or_else(|_| {
            filtered_status_snapshot("Skill results cannot be displayed safely", active.filter)
        });
    let selectable = !matches!(active.status, SkillReferenceSearchStatus::Failed(_))
        && active
            .visible
            .values()
            .any(|candidate| matches!(candidate.availability(), SkillAvailability::Enabled));
    let refreshed = overlay.refresh(active.token, snapshot).is_ok();
    if refreshed {
        let _ = overlay.set_acceptance_enabled(active.token, selectable);
    }
    refreshed
}

fn filtered_status_snapshot(message: &str, filter: SkillFilter) -> PanelSnapshot {
    PanelSnapshot::new(
        "Skills",
        vec![SelectionEntry::disabled(
            "status",
            display_candidate_text(message),
            None,
            "not selectable",
        )],
    )
    .and_then(|snapshot| snapshot.with_filter_bar(FILTER_LABELS, filter.index()))
    .expect("built-in skill status panel is valid")
}

fn status_message(status: &SkillReferenceSearchStatus, filter: SkillFilter) -> String {
    match status {
        SkillReferenceSearchStatus::Complete => {
            if matches!(filter, SkillFilter::All) {
                "No matching skills".to_owned()
            } else {
                format!("No matching {} skills", filter.label().to_lowercase())
            }
        },
        SkillReferenceSearchStatus::Incomplete(reason)
        | SkillReferenceSearchStatus::Failed(reason) => display_candidate_text(reason),
    }
}

impl SkillFilter {
    const fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Workspace => 1,
            Self::User => 2,
            Self::System => 3,
            Self::Admin => 4,
        }
    }

    const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::All),
            1 => Some(Self::Workspace),
            2 => Some(Self::User),
            3 => Some(Self::System),
            4 => Some(Self::Admin),
            _ => None,
        }
    }

    const fn label(self) -> &'static str {
        FILTER_LABELS[self.index()]
    }

    const fn matches(self, scope: SkillReferenceScope) -> bool {
        matches!(self, Self::All)
            || matches!(
                (self, scope),
                (Self::Workspace, SkillReferenceScope::Workspace)
                    | (Self::User, SkillReferenceScope::User)
                    | (Self::System, SkillReferenceScope::System)
                    | (Self::Admin, SkillReferenceScope::Admin)
            )
    }
}

const fn scope_label(scope: SkillReferenceScope) -> &'static str {
    match scope {
        SkillReferenceScope::Workspace => "Workspace",
        SkillReferenceScope::User => "User",
        SkillReferenceScope::System => "System",
        SkillReferenceScope::Admin => "Admin",
    }
}

#[cfg(test)]
mod tests;
