//! The existing draft/submission lane owns image identities and aggregate ownership.

use std::ops::Range;

use yo_core::{ImagePreparationRequest, ImagePreparationUpdate, InputImage, InputImageSnapshot};

use super::{StateEffect, StateError, TuiState};
use crate::{command::attachment_argument, prompt::workspace_reference::WorkspaceEdit};

const MAX_OWNED_PNG_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct PendingImage {
    id: u64,
    revision: u64,
    draft: String,
    command: Range<usize>,
}

impl TuiState {
    pub(in crate::runner) fn enable_image_preparation(&mut self) {
        self.image_preparation_enabled = true;
    }

    pub(in crate::runner) fn image_preparation_is_current(&self) -> bool {
        !self.has_pending_request()
            && self.pending_model_selection.is_none()
            && self.reserved_model_selection.is_none()
            && !self.new_session_requested
            && !self.fork_session_requested
            && self.resume_session_requested.is_none()
            && self.pending_image.as_ref().is_some_and(|pending| {
                pending.revision == self.prompt_assist.image_revision()
                    && pending.draft == self.editor.text()
            })
    }

    pub(super) fn prepare_image_command(&mut self, draft: &str) -> Result<StateEffect, StateError> {
        if self.editor.text().is_empty() {
            self.editor.replace_range(0..0, draft);
        }
        let Some((command, source)) = attachment_argument(draft) else {
            return Ok(StateEffect::Redraw);
        };
        if !self.image_preparation_enabled
            || self.preview_mode
            || self.has_pending_request()
            || self.pending_model_selection.is_some()
            || self.reserved_model_selection.is_some()
        {
            self.chat.push_notice(
                "Image attachment is unavailable in this prompt; your draft was preserved."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        if source.as_os_str().is_empty() {
            self.chat.push_notice("Use /attach PATH to prepare a local PNG or JPEG. Add it on a new final draft line to keep existing text and images.".to_owned())?;
            return Ok(StateEffect::Redraw);
        }
        if self.pending_image.is_some() || !self.pending_submissions.is_empty() {
            self.chat.push_notice(
                "Wait for pending preparation or admission before attaching another image."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        let owned = self.owned_image_bytes();
        if self.prompt_assist.image_occurrences().len() >= InputImage::MAX_OCCURRENCES
            || !can_reserve_image(owned)
        {
            self.chat.push_notice("Image ownership limit reached (16 per input, 64 MiB across drafts and queue); your draft was preserved.".to_owned())?;
            return Ok(StateEffect::Redraw);
        }
        let digests = self
            .prompt_assist
            .image_occurrences()
            .iter()
            .chain(self.follow_ups.iter().flat_map(|input| input.images()))
            .chain(
                self.pending_submissions
                    .iter()
                    .flat_map(|submission| submission.input().images()),
            )
            .map(|image| image.snapshot().sha256().to_owned())
            .collect::<Vec<_>>();
        self.prompt_assist.retain_image_thumbnails(&digests);
        self.next_image_id = self
            .next_image_id
            .checked_add(1)
            .ok_or(StateError::ItemIdOverflow)?;
        let revision = self.prompt_assist.image_revision();
        let id = self.next_image_id;
        self.pending_image = Some(PendingImage {
            id,
            revision,
            draft: draft.to_owned(),
            command,
        });
        self.chat.push_notice(
            "Preparing image… Editing the draft cancels this attachment.".to_owned(),
        )?;
        Ok(StateEffect::PrepareImage(ImagePreparationRequest {
            id,
            revision,
            source,
        }))
    }

    pub(in crate::runner) fn observe_image_preparation(
        &mut self,
        update: ImagePreparationUpdate,
    ) -> Result<bool, StateError> {
        if !self
            .pending_image
            .as_ref()
            .is_some_and(|pending| pending.id == update.id && pending.revision == update.revision)
        {
            return Ok(false);
        }
        let current = self.image_preparation_is_current();
        let pending = self
            .pending_image
            .take()
            .expect("the update matched its pending image");
        if !current {
            return Ok(false);
        }
        let prepared = match update.result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.chat
                    .push_notice(format!("{} Your draft was preserved.", error.message()))?;
                return Ok(true);
            },
        };
        if self.owned_image_bytes()
            > MAX_OWNED_PNG_BYTES.saturating_sub(prepared.image().snapshot().png().len())
        {
            self.chat.push_notice(
                "Image ownership limit reached; your draft was preserved.".to_owned(),
            )?;
            return Ok(true);
        }
        let original = match self.prompt_assist.input(&pending.draft) {
            Ok(input) => input,
            Err(_) => {
                self.chat
                    .push_notice("Draft annotations changed; attach the image again.".to_owned())?;
                return Ok(true);
            },
        };
        let old_cursor = self.editor.cursor_byte_index();
        self.editor
            .replace_range(pending.command.clone(), InputImage::PROJECTION);
        let edit = WorkspaceEdit::between(
            &pending.draft,
            old_cursor,
            self.editor.text(),
            self.editor.cursor_byte_index(),
        );
        let _ = self.prompt_assist.prompt_changed(
            &self.editor,
            &mut self.overlay,
            edit.as_ref(),
            false,
        );
        let attached = self
            .prompt_assist
            .attach_image(
                &prepared,
                pending.command.start..pending.command.start + InputImage::PROJECTION.len(),
            )
            .and_then(|()| self.prompt_assist.input(self.editor.text()));
        if let Err(error) = attached {
            self.editor
                .replace_range(0..self.editor.text().len(), original.as_str());
            self.prompt_assist
                .restore_input(&original, &mut self.overlay);
            self.chat.push_notice(format!(
                "Image was not attached: {error}. Your draft was preserved."
            ))?;
            return Ok(true);
        }
        let image = prepared.image();
        let source = image.display();
        self.chat.push_notice(format!(
            "Image attached: source {} × {}; transmission {} × {} RGBA8 PNG ({} bytes). Preview is a {} × {} thumbnail; the source file is unchanged.",
            source.and_then(|display| display.source_width).unwrap_or(0), source.and_then(|display| display.source_height).unwrap_or(0),
            image.snapshot().width(), image.snapshot().height(), image.snapshot().png().len(), prepared.thumbnail_width(), prepared.thumbnail_height(),
        ))?;
        Ok(true)
    }

    fn owned_image_bytes(&self) -> usize {
        self.prompt_assist
            .image_occurrences()
            .iter()
            .chain(self.follow_ups.iter().flat_map(|input| input.images()))
            // The pending submission references the same still-visible draft/queue occurrence.
            // Its clone does not create a second editable occurrence or new owned PNG bytes.
            .map(|image| image.snapshot().png().len())
            .fold(0, usize::saturating_add)
    }
}

fn can_reserve_image(owned: usize) -> bool {
    owned
        .checked_add(InputImageSnapshot::MAX_BYTES)
        .is_some_and(|bytes| bytes <= MAX_OWNED_PNG_BYTES)
}

#[cfg(test)]
mod tests;
