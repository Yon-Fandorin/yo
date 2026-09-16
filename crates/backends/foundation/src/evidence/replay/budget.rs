pub(super) const MAX_REPLAY_ITEMS: usize = 4_096;
pub(in crate::evidence) const MAX_REPLAY_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub(in crate::evidence) const MAX_REPLAY_CONTRACT_BYTES: usize = 1024 * 1024;
pub(in crate::evidence) const MAX_REPLAY_DELTA_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_REPLAY_PREFIX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelReplayBudget {
    encoded_prefix_bytes: usize,
    prefix_items: usize,
}

impl ModelReplayBudget {
    pub(super) const fn from_prefix(encoded_prefix_bytes: usize, prefix_items: usize) -> Self {
        Self {
            encoded_prefix_bytes,
            prefix_items,
        }
    }

    pub fn accepts_item_lengths(&self, item_lengths: &[usize]) -> bool {
        self.encoded_len_with_item_lengths(item_lengths)
            .is_some_and(|bytes| bytes <= MAX_REPLAY_DELTA_BYTES)
    }

    pub fn encoded_len_with_item_lengths(&self, item_lengths: &[usize]) -> Option<usize> {
        let total_items = self.prefix_items.checked_add(item_lengths.len())?;
        if total_items > MAX_REPLAY_ITEMS {
            return None;
        }
        item_lengths
            .iter()
            .try_fold(
                (self.encoded_prefix_bytes, self.prefix_items),
                |(bytes, items), item_bytes| {
                    bytes
                        .checked_add(usize::from(items != 0))?
                        .checked_add(*item_bytes)
                        .map(|bytes| (bytes, items + 1))
                },
            )
            .map(|(bytes, _)| bytes)
    }
}
