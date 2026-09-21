use std::{mem, ops::Range};

use yo_core::{InputImage, InputReference, UserInput};

use super::{StateEffect, TuiState};
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    prompt::workspace_reference::WorkspaceEdit,
    runner::{ExternalEditorImportError, ExternalEditorSnapshot, view::ObservabilityView},
};

impl TuiState {
    pub(in crate::runner) fn external_editor_snapshot(&self) -> Option<ExternalEditorSnapshot> {
        self.external_editor_snapshot.clone()
    }

    pub(in crate::runner) fn take_external_editor_request(&mut self) -> bool {
        mem::take(&mut self.external_editor_requested)
    }

    pub(in crate::runner) fn report_external_editor_failure(&mut self, detail: String) {
        self.external_editor_requested = false;
        self.external_editor_snapshot = None;
        let _ = self.chat.push_notice(format!(
            "External editor was not applied: {detail}. Your draft was preserved."
        ));
    }

    pub(super) fn handle_external_editor_key(
        &mut self,
        input: &InputEvent,
    ) -> Result<Option<StateEffect>, super::StateError> {
        if !matches!(
            input,
            InputEvent::Key(key)
                if key.action == KeyAction::Press
                    && key.modifiers == KeyModifiers::CONTROL
                    && matches!(key.code, KeyCode::Character('g' | 'G'))
        ) {
            return Ok(None);
        }
        if self.views.active() != ObservabilityView::Chat
            || self.has_pending_request()
            || self.overlay.panel().is_some()
        {
            return Ok(None);
        }
        let input = match self.prompt_assist.input(self.editor.text()) {
            Ok(input) => input,
            Err(error) => {
                self.chat.push_notice(format!(
                    "External editor unavailable: {error}. Your draft was preserved."
                ))?;
                return Ok(Some(StateEffect::Redraw));
            },
        };
        self.external_editor_generation = self.external_editor_generation.saturating_add(1);
        self.external_editor_snapshot = Some(ExternalEditorSnapshot::new(
            input,
            self.editor.cursor_byte_index(),
            self.external_editor_generation,
        ));
        self.external_editor_requested = true;
        Ok(Some(StateEffect::Exit))
    }

    pub(in crate::runner) fn import_external_editor_result(
        &mut self,
        snapshot: &ExternalEditorSnapshot,
        text: String,
    ) -> Result<(), ExternalEditorImportError> {
        let Some(pending) = self.external_editor_snapshot.as_ref() else {
            return Err(ExternalEditorImportError::NoRequest);
        };
        if pending.generation() != snapshot.generation()
            || pending != snapshot
            || self.views.active() != ObservabilityView::Chat
            || self.has_pending_request()
            || self.overlay.panel().is_some()
            || self.editor.cursor_byte_index() != snapshot.cursor_byte_index()
            || self.prompt_assist.input(self.editor.text()).ok().as_ref() != Some(snapshot.input())
        {
            return Err(ExternalEditorImportError::StaleDraft);
        }

        let old_text = self.editor.text().to_owned();
        if old_text == text {
            self.external_editor_snapshot = None;
            return Ok(());
        }
        let edit =
            WorkspaceEdit::between(&old_text, snapshot.cursor_byte_index(), &text, text.len())
                .ok_or(ExternalEditorImportError::InvalidDraft)?;
        if edit_intersects_annotations(&edit, snapshot.input()) {
            return Err(ExternalEditorImportError::AmbiguousAnnotations);
        }
        let mapped = map_input(snapshot.input(), &text, &edit)?;
        self.editor.replace_range_undoable(0..old_text.len(), &text);
        self.prompt_assist.restore_input(&mapped, &mut self.overlay);
        let command_eligible =
            self.views.active() == ObservabilityView::Chat && !self.has_pending_request();
        self.command_palette.sync(
            self.editor.text(),
            self.editor.cursor_byte_index(),
            &mut self.overlay,
            command_eligible,
        );
        self.external_editor_snapshot = None;
        Ok(())
    }
}

fn edit_intersects_annotations(edit: &WorkspaceEdit, input: &UserInput) -> bool {
    input
        .references()
        .iter()
        .any(|reference| ranges_overlap(edit.old_range(), reference.span()))
        || input
            .images()
            .iter()
            .any(|image| ranges_overlap(edit.old_range(), image.span()))
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && left.end > right.start
}

fn map_input(
    input: &UserInput,
    text: &str,
    edit: &WorkspaceEdit,
) -> Result<UserInput, ExternalEditorImportError> {
    let references = input
        .references()
        .iter()
        .map(|reference| {
            let span = shift_span(reference.span(), edit)?;
            if let Some(workspace) = reference.workspace_reference() {
                Ok(InputReference::workspace(span, workspace.clone()))
            } else if let Some(skill) = reference.skill_reference() {
                Ok(InputReference::skill(span, skill.clone()))
            } else {
                Err(ExternalEditorImportError::InvalidDraft)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let images = input
        .images()
        .iter()
        .map(|image| {
            let span = shift_span(image.span(), edit)?;
            let mut mapped =
                InputImage::new(span, image.source_byte_length(), image.snapshot().clone())
                    .map_err(|_| ExternalEditorImportError::InvalidDraft)?;
            if let Some(display) = image.display() {
                mapped = mapped
                    .with_display(display.clone())
                    .map_err(|_| ExternalEditorImportError::InvalidDraft)?;
            }
            Ok(mapped)
        })
        .collect::<Result<Vec<_>, ExternalEditorImportError>>()?;
    UserInput::with_references(text.to_owned(), references)
        .map_err(|_| ExternalEditorImportError::InvalidDraft)?
        .with_images(images)
        .map_err(|_| ExternalEditorImportError::InvalidDraft)
}

fn shift_span(
    span: &Range<usize>,
    edit: &WorkspaceEdit,
) -> Result<Range<usize>, ExternalEditorImportError> {
    if ranges_overlap(edit.old_range(), span) {
        return Err(ExternalEditorImportError::AmbiguousAnnotations);
    }
    if edit.old_range().end <= span.start {
        let delta = isize::try_from(edit.new_range().len()).unwrap_or(isize::MAX)
            - isize::try_from(edit.old_range().len()).unwrap_or(isize::MAX);
        return Ok(span.start.saturating_add_signed(delta)..span.end.saturating_add_signed(delta));
    }
    Ok(span.clone())
}

#[cfg(test)]
mod tests;
