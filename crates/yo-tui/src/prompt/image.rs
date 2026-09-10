//! Draft image occurrences and already-prepared thumbnails; no decoder or filesystem access.

use std::{collections::BTreeMap, ops::Range, sync::Arc};

use yo_core::{InputImage, PreparedImageAttachment, UserInput, UserInputError};

use super::workspace_reference::WorkspaceEdit;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImageThumbnail {
    pub(crate) png: Arc<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Default)]
pub(super) struct ImageAssist {
    images: Vec<InputImage>,
    last_text: String,
    thumbnails: BTreeMap<String, ImageThumbnail>,
}

impl ImageAssist {
    pub(super) fn images(&self) -> &[InputImage] {
        &self.images
    }
    pub(super) fn thumbnail(&self) -> Option<&ImageThumbnail> {
        self.images
            .last()
            .and_then(|image| self.thumbnails.get(image.snapshot().sha256()))
    }
    pub(super) fn restore(&mut self, input: &UserInput) {
        self.images = input.images().to_vec();
        self.last_text = input.as_str().to_owned();
    }
    pub(super) fn update(&mut self, text: &str, edit: Option<&WorkspaceEdit>) {
        let inferred =
            WorkspaceEdit::between(&self.last_text, self.last_text.len(), text, text.len());
        if let Some(edit) = edit.or(inferred.as_ref()) {
            let delta = edit.new.len() as isize - edit.old.len() as isize;
            self.images.retain_mut(|image| {
                let mut span = image.span().clone();
                if edit.old.end <= span.start {
                    span = span.start.saturating_add_signed(delta)
                        ..span.end.saturating_add_signed(delta);
                } else if edit.old.start < span.end {
                    return false;
                }
                if text.get(span.clone()) != Some(InputImage::PROJECTION) {
                    return false;
                }
                *image = relocate(image, span)
                    .expect("moving a validated occurrence preserves source evidence");
                true
            });
        }
        self.last_text = text.to_owned();
    }
    pub(super) fn attach(
        &mut self,
        prepared: &PreparedImageAttachment,
        span: Range<usize>,
    ) -> Result<(), UserInputError> {
        self.images.push(relocate(prepared.image(), span)?);
        self.images.sort_by_key(|image| image.span().start);
        self.thumbnails.insert(
            prepared.snapshot_digest().to_owned(),
            ImageThumbnail {
                png: prepared.thumbnail_png().clone(),
                width: prepared.thumbnail_width(),
                height: prepared.thumbnail_height(),
            },
        );
        Ok(())
    }
    pub(super) fn retain_thumbnails(&mut self, digests: &[String]) {
        self.thumbnails.retain(|digest, _| digests.contains(digest));
    }
}

fn relocate(image: &InputImage, span: Range<usize>) -> Result<InputImage, UserInputError> {
    let mut moved = InputImage::new(span, image.source_byte_length(), image.snapshot().clone())?;
    if let Some(display) = image.display() {
        moved = moved.with_display(display.clone())?;
    }
    Ok(moved)
}
