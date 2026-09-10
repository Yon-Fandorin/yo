//! Ordered message state for agent conversation rendering.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "transcript state lands before its layout and shell consumers"
    )
)]

use std::{collections::HashMap, ops::Range};

mod activity;
mod layout;
use activity::ActivityPresentation;
pub(crate) use activity::FileChangeView;
pub use activity::{
    AssistantRenderInput, AssistantRenderer, DocumentRenderInput, DocumentRenderer, LinkResolver,
    ToolRenderInput, ToolRenderer, TranscriptActivityOutcome,
};
mod viewport;

pub(crate) use layout::{
    MarkdownStyles, TranscriptActivityStyles, TranscriptLayoutConfig, TranscriptLayoutConfigError,
    TranscriptMeasure, TranscriptMeasureError, TranscriptPaintError, TranscriptRenderError,
    TranscriptRenderFrame, TranscriptStyles, measure, measure_slice, paint_prepared,
    paint_prepared_commands, prepare, prepare_slice, render, render_commands, render_slice,
};
pub(crate) use viewport::{TranscriptScrollCommand, TranscriptViewMode, TranscriptViewState};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TranscriptItemId(u64);

impl TranscriptItemId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn get(self) -> u64 {
        self.0
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
    markdown: bool,
    assistant_footer_start: Option<usize>,
    activity: Option<ActivityPresentation>,
    text: String,
}

impl TranscriptMessage {
    pub(crate) const fn role(&self) -> MessageRole {
        self.role
    }

    pub(crate) const fn is_markdown(&self) -> bool {
        self.markdown
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct TranscriptSlice<'transcript> {
    items: &'transcript [TranscriptItem],
    has_visible_predecessor: bool,
}

impl<'transcript> TranscriptSlice<'transcript> {
    pub(crate) const fn items(self) -> &'transcript [TranscriptItem] {
        self.items
    }

    pub(crate) const fn has_visible_predecessor(self) -> bool {
        self.has_visible_predecessor
    }
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
                markdown: false,
                assistant_footer_start: None,
                activity: None,
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
                markdown: false,
                assistant_footer_start: None,
                activity: None,
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
    UnfinishedItem(TranscriptItemId),
    NotActivity(TranscriptItemId),
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

    pub(crate) fn slice(&self, range: Range<usize>) -> TranscriptSlice<'_> {
        assert!(
            range.start <= range.end && range.end <= self.items.len(),
            "transcript slice must stay inside the ordered item list"
        );
        TranscriptSlice {
            items: &self.items[range.clone()],
            has_visible_predecessor: self.items[..range.start]
                .iter()
                .any(transcript_item_is_visible),
        }
    }

    pub(crate) fn all(&self) -> TranscriptSlice<'_> {
        self.slice(0..self.items.len())
    }

    pub(crate) fn suffix(&self, start: usize) -> TranscriptSlice<'_> {
        self.slice(start..self.items.len())
    }

    pub(crate) fn plain_output(
        &self,
        config: &TranscriptLayoutConfig,
    ) -> Result<Option<String>, TranscriptMeasureError> {
        self.plain_output_slice(self.all(), config)
    }

    pub(crate) fn plain_output_slice(
        &self,
        slice: TranscriptSlice<'_>,
        config: &TranscriptLayoutConfig,
    ) -> Result<Option<String>, TranscriptMeasureError> {
        layout::plain_output(slice, config)
    }

    /// Copies immutable presentation only; execution identities remain with the source.
    pub(crate) fn push_final_copy(
        &mut self,
        id: TranscriptItemId,
        source: &TranscriptItem,
    ) -> Result<(), TranscriptStateError> {
        if source.phase != TranscriptPhase::Final {
            return Err(TranscriptStateError::UnfinishedItem(source.id));
        }
        let mut item = source.clone();
        item.id = id;
        item.revision = 0;
        self.push(item)
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

    pub(crate) fn start_markdown_assistant(
        &mut self,
        id: TranscriptItemId,
    ) -> Result<(), TranscriptStateError> {
        let mut item = TranscriptItem::streaming_assistant(id);
        let TranscriptBody::Message(message) = &mut item.body;
        message.markdown = true;
        self.push(item)
    }

    pub(crate) fn append_outcome_footer(
        &mut self,
        id: TranscriptItemId,
        footer: &str,
    ) -> Result<(), TranscriptStateError> {
        self.append_text(id, footer)?;
        let TranscriptBody::Message(message) = &mut self.item_mut(id)?.body;
        if message.markdown && message.activity.is_none() {
            message.assistant_footer_start = Some(message.text.len() - footer.len());
        }
        Ok(())
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
        self.replace_text_changed(id, text).map(|_| ())
    }

    pub(crate) fn replace_text_changed(
        &mut self,
        id: TranscriptItemId,
        text: String,
    ) -> Result<bool, TranscriptStateError> {
        let item = self.item_mut(id)?;
        if item.phase == TranscriptPhase::Final {
            return Err(TranscriptStateError::FinalItem(id));
        }
        let TranscriptBody::Message(message) = &mut item.body;
        if message.text == text {
            return Ok(false);
        }

        item.revision = item
            .revision
            .checked_add(1)
            .ok_or(TranscriptStateError::RevisionOverflow(id))?;
        message.text = text;
        Ok(true)
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

fn transcript_item_is_visible(item: &TranscriptItem) -> bool {
    let TranscriptBody::Message(message) = item.body();
    !message.text().is_empty() || item.phase() == TranscriptPhase::Final
}

#[cfg(test)]
mod tests;
