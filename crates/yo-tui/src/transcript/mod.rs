//! Ordered message state for agent conversation rendering.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "transcript state lands before its layout and shell consumers"
    )
)]

use std::collections::HashMap;

mod layout;
mod viewport;

pub(crate) use layout::{
    TranscriptLayoutConfig, TranscriptLayoutConfigError, TranscriptMeasure, TranscriptMeasureError,
    TranscriptPaintError, TranscriptRenderError, TranscriptRenderFrame, TranscriptStyles, measure,
    paint_prepared, prepare, render,
};
pub(crate) use viewport::{TranscriptScrollCommand, TranscriptViewMode, TranscriptViewState};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TranscriptItemId(u64);

impl TranscriptItemId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptPhase {
    Streaming,
    Final,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptMessage {
    role: MessageRole,
    text: String,
}

impl TranscriptMessage {
    pub(crate) const fn role(&self) -> MessageRole {
        self.role
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptBody {
    Message(TranscriptMessage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptItem {
    id: TranscriptItemId,
    revision: u64,
    phase: TranscriptPhase,
    body: TranscriptBody,
}

impl TranscriptItem {
    pub(crate) const fn id(&self) -> TranscriptItemId {
        self.id
    }

    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) const fn phase(&self) -> TranscriptPhase {
        self.phase
    }

    pub(crate) const fn body(&self) -> &TranscriptBody {
        &self.body
    }

    fn user(id: TranscriptItemId, text: String) -> Self {
        Self {
            id,
            revision: 0,
            phase: TranscriptPhase::Final,
            body: TranscriptBody::Message(TranscriptMessage {
                role: MessageRole::User,
                text,
            }),
        }
    }

    fn streaming_assistant(id: TranscriptItemId) -> Self {
        Self {
            id,
            revision: 0,
            phase: TranscriptPhase::Streaming,
            body: TranscriptBody::Message(TranscriptMessage {
                role: MessageRole::Assistant,
                text: String::new(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptStateError {
    DuplicateId(TranscriptItemId),
    UnknownId(TranscriptItemId),
    FinalItem(TranscriptItemId),
    RevisionOverflow(TranscriptItemId),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TranscriptState {
    items: Vec<TranscriptItem>,
    indexes: HashMap<TranscriptItemId, usize>,
}

impl TranscriptState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn items(&self) -> &[TranscriptItem] {
        &self.items
    }

    pub(crate) fn push_user(
        &mut self,
        id: TranscriptItemId,
        text: String,
    ) -> Result<(), TranscriptStateError> {
        self.push(TranscriptItem::user(id, text))
    }

    pub(crate) fn start_assistant(
        &mut self,
        id: TranscriptItemId,
    ) -> Result<(), TranscriptStateError> {
        self.push(TranscriptItem::streaming_assistant(id))
    }

    pub(crate) fn append_text(
        &mut self,
        id: TranscriptItemId,
        text: &str,
    ) -> Result<(), TranscriptStateError> {
        let item = self.item_mut(id)?;
        if item.phase == TranscriptPhase::Final {
            return Err(TranscriptStateError::FinalItem(id));
        }
        if text.is_empty() {
            return Ok(());
        }

        let revision = item
            .revision
            .checked_add(1)
            .ok_or(TranscriptStateError::RevisionOverflow(id))?;
        let TranscriptBody::Message(message) = &mut item.body;
        message.text.push_str(text);
        item.revision = revision;
        Ok(())
    }

    pub(crate) fn replace_text(
        &mut self,
        id: TranscriptItemId,
        text: String,
    ) -> Result<(), TranscriptStateError> {
        let item = self.item_mut(id)?;
        if item.phase == TranscriptPhase::Final {
            return Err(TranscriptStateError::FinalItem(id));
        }
        let TranscriptBody::Message(message) = &mut item.body;
        if message.text == text {
            return Ok(());
        }

        item.revision = item
            .revision
            .checked_add(1)
            .ok_or(TranscriptStateError::RevisionOverflow(id))?;
        message.text = text;
        Ok(())
    }

    pub(crate) fn finalize(&mut self, id: TranscriptItemId) -> Result<(), TranscriptStateError> {
        let item = self.item_mut(id)?;
        if item.phase == TranscriptPhase::Final {
            return Err(TranscriptStateError::FinalItem(id));
        }

        item.revision = item
            .revision
            .checked_add(1)
            .ok_or(TranscriptStateError::RevisionOverflow(id))?;
        item.phase = TranscriptPhase::Final;
        Ok(())
    }

    fn push(&mut self, item: TranscriptItem) -> Result<(), TranscriptStateError> {
        if self.indexes.contains_key(&item.id) {
            return Err(TranscriptStateError::DuplicateId(item.id));
        }

        let id = item.id;
        let index = self.items.len();
        self.items.push(item);
        let previous = self.indexes.insert(id, index);
        debug_assert!(
            previous.is_none(),
            "duplicate IDs are checked before insertion"
        );
        Ok(())
    }

    fn item_mut(
        &mut self,
        id: TranscriptItemId,
    ) -> Result<&mut TranscriptItem, TranscriptStateError> {
        let index = self
            .indexes
            .get(&id)
            .copied()
            .ok_or(TranscriptStateError::UnknownId(id))?;
        Ok(&mut self.items[index])
    }
}

#[cfg(test)]
mod tests;
