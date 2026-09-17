use std::io::{Error, Result as IoResult, Write};

use serde::{Deserialize, Serialize};

use crate::{InputImageSnapshot, JournalSequence, ModelInputPart, ModelReplayItem};

pub(crate) const CONTEXT_ARTIFACT_PROFILE: &str = "yo.context-artifact-receipt/v1alpha1";
pub(super) const MAX_CONTEXT_ITEMS: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextRetainedGroup {
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
    fork_import: Option<(JournalSequence, usize)>,
    private_epochs: Vec<u64>,
}
impl ContextRetainedGroup {
    pub(crate) fn try_new(
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> Result<Self, &'static str> {
        if first_sequence > last_sequence || items.is_empty() {
            return Err("context retained group has an invalid source range or no items");
        }
        Ok(Self {
            first_sequence,
            last_sequence,
            items,
            fork_import: None,
            private_epochs: Vec::new(),
        })
    }

    pub(crate) fn try_imported(
        seed_sequence: JournalSequence,
        group_index: usize,
        items: Vec<ModelReplayItem>,
        private_epochs: Vec<u64>,
    ) -> Result<Self, &'static str> {
        let private_count = items
            .iter()
            .filter(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }))
            .count();
        if seed_sequence.get() == 0
            || group_index >= MAX_CONTEXT_ITEMS
            || items.is_empty()
            || items.len() > MAX_CONTEXT_ITEMS
            || private_count != private_epochs.len()
            || private_epochs.contains(&0)
        {
            return Err(
                "imported retained group has invalid coordinates, items, or private epochs",
            );
        }
        Ok(Self {
            first_sequence: seed_sequence,
            last_sequence: seed_sequence,
            items,
            fork_import: Some((seed_sequence, group_index)),
            private_epochs,
        })
    }

    pub(crate) const fn fork_import(&self) -> Option<(JournalSequence, usize)> {
        self.fork_import
    }

    pub(crate) fn private_epochs(&self) -> &[u64] {
        &self.private_epochs
    }

    pub(crate) const fn first_sequence(&self) -> JournalSequence {
        self.first_sequence
    }

    pub(crate) const fn last_sequence(&self) -> JournalSequence {
        self.last_sequence
    }

    pub(crate) fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextArtifactReceipt {
    content_hash: String,
    byte_count: u64,
    media_kind: String,
    source_context_epoch: u64,
    source_journal_sequence: JournalSequence,
}

impl ContextArtifactReceipt {
    pub(crate) fn try_new(
        content_hash: impl Into<String>,
        byte_count: u64,
        media_kind: impl Into<String>,
        source_context_epoch: u64,
        source_journal_sequence: JournalSequence,
    ) -> Result<Self, &'static str> {
        let record = Self {
            content_hash: content_hash.into(),
            byte_count,
            media_kind: media_kind.into(),
            source_context_epoch,
            source_journal_sequence,
        };
        if !valid_sha256(&record.content_hash)
            || record.byte_count == 0
            || record.source_context_epoch == 0
            || !valid_bounded_ascii(&record.media_kind, 128)
        {
            return Err("context artifact receipt is invalid");
        }
        Ok(record)
    }

    pub(crate) fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub(crate) const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    pub(crate) fn media_kind(&self) -> &str {
        &self.media_kind
    }

    pub(crate) const fn source_context_epoch(&self) -> u64 {
        self.source_context_epoch
    }

    pub(crate) const fn source_journal_sequence(&self) -> JournalSequence {
        self.source_journal_sequence
    }
}

/// Local immutable source coordinates; ancestor qualifications remain fork provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ContextImageSource {
    ReplayDelta {
        sequence: u64,
        item_index: u32,
        part_index: u32,
    },
    RetainedCheckpoint {
        sequence: u64,
        group_index: u32,
        item_index: u32,
        part_index: u32,
    },
    InitialForkSeed {
        sequence: u64,
        group_index: u32,
        item_index: u32,
        part_index: u32,
    },
}

impl ContextImageSource {
    pub(crate) fn sequence(&self) -> u64 {
        match self {
            Self::ReplayDelta { sequence, .. }
            | Self::RetainedCheckpoint { sequence, .. }
            | Self::InitialForkSeed { sequence, .. } => *sequence,
        }
    }
    fn valid(&self) -> bool {
        let (sequence, group, item, part) = match self {
            Self::ReplayDelta {
                sequence,
                item_index,
                part_index,
            } => (*sequence, 0, *item_index, *part_index),
            Self::RetainedCheckpoint {
                sequence,
                group_index,
                item_index,
                part_index,
            }
            | Self::InitialForkSeed {
                sequence,
                group_index,
                item_index,
                part_index,
            } => (*sequence, *group_index, *item_index, *part_index),
        };
        sequence > 0 && group < 4096 && item < 4096 && part < 33
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextImageLoss {
    content_hash: String,
    byte_count: u64,
    width: u32,
    height: u32,
    source_context_epoch: u64,
    source: ContextImageSource,
}

impl ContextImageLoss {
    pub(crate) fn from_snapshot(
        snapshot: &InputImageSnapshot,
        source_context_epoch: u64,
        source: ContextImageSource,
    ) -> Result<Self, &'static str> {
        let loss = Self {
            content_hash: snapshot.sha256().to_owned(),
            byte_count: snapshot.png().len() as u64,
            width: snapshot.width(),
            height: snapshot.height(),
            source_context_epoch,
            source,
        };
        loss.validate()?;
        Ok(loss)
    }
    fn validate(&self) -> Result<(), &'static str> {
        if !valid_sha256(&self.content_hash)
            || self.byte_count == 0
            || self.byte_count > InputImageSnapshot::MAX_BYTES as u64
            || self.width == 0
            || self.height == 0
            || self.width > InputImageSnapshot::MAX_SIDE
            || self.height > InputImageSnapshot::MAX_SIDE
            || u64::from(self.width) * u64::from(self.height) > InputImageSnapshot::MAX_PIXELS
            || self.source_context_epoch == 0
            || !self.source.valid()
        {
            return Err("input image loss has invalid identity or source bounds");
        }
        Ok(())
    }
    pub(crate) fn source(&self) -> &ContextImageSource {
        &self.source
    }
    /// Derives runtime-owned receipts in flattened occurrence order, never from model prose.
    pub(crate) fn for_items(
        items: &[ModelReplayItem],
        source_epoch: u64,
        mut source: impl FnMut(u32, u32) -> ContextImageSource,
    ) -> Result<Vec<Self>, &'static str> {
        let mut losses = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            if let ModelReplayItem::MultimodalUser { parts } = item {
                for (part_index, part) in parts.iter().enumerate() {
                    if let ModelInputPart::Image { snapshot } = part {
                        losses.push(Self::from_snapshot(
                            snapshot,
                            source_epoch,
                            source(
                                u32::try_from(item_index)
                                    .map_err(|_| "image item index overflow")?,
                                u32::try_from(part_index)
                                    .map_err(|_| "image part index overflow")?,
                            ),
                        )?);
                    }
                }
            }
        }
        Ok(losses)
    }
}

pub(crate) fn validate_image_losses(losses: &[ContextImageLoss]) -> Result<(), &'static str> {
    if losses.len() > 64 {
        return Err("checkpoint image loss count exceeds 64");
    }
    #[derive(Serialize)]
    struct Loss<'a> {
        kind: &'static str,
        #[serde(flatten)]
        loss: &'a ContextImageLoss,
    }
    struct Budget(usize);
    impl Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|len| *len <= 64 * 1024)
                .ok_or_else(|| Error::other("image loss array exceeds 64 KiB"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }
    for loss in losses {
        loss.validate()?;
    }
    serde_json::to_writer(
        Budget(0),
        &losses
            .iter()
            .map(|loss| Loss {
                kind: "image_input_summarized",
                loss,
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|_| "image loss array exceeds 64 KiB")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ContextLoss {
    ImageInputSummarized(ContextImageLoss),
    VisiblePrefixSummarized {
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
    },
    ProviderPrivateDropped {
        schema: String,
        byte_count: u64,
        source_journal_sequence: JournalSequence,
    },
}

impl ContextLoss {
    pub(crate) fn visible_prefix_summarized(
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
    ) -> Result<Self, &'static str> {
        if first_sequence > last_sequence {
            return Err("summarized context loss range is invalid");
        }
        Ok(Self::VisiblePrefixSummarized {
            first_sequence,
            last_sequence,
        })
    }

    pub(crate) fn provider_private_dropped(
        schema: impl Into<String>,
        byte_count: u64,
        source_journal_sequence: JournalSequence,
    ) -> Result<Self, &'static str> {
        let schema = schema.into();
        if !valid_bounded_ascii(&schema, 128) || byte_count == 0 {
            return Err("provider-private context loss is invalid");
        }
        Ok(Self::ProviderPrivateDropped {
            schema,
            byte_count,
            source_journal_sequence,
        })
    }
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn valid_bounded_ascii(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.is_ascii()
}
