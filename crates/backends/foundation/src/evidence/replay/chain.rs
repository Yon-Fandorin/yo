use std::collections::BTreeSet;

use super::{
    budget::{
        MAX_REPLAY_DELTA_BYTES, MAX_REPLAY_ITEMS, MAX_REPLAY_PREFIX_BYTES, ModelReplayBudget,
    },
    contract::ModelReplayContract,
    item::{ModelReplayItem, is_valid_item},
    validation::{
        encoded_prefix_len, replay_delta_is_valid, validate_replay_delta, validate_replay_items,
    },
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplayDelta {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
}

impl ModelReplayDelta {
    pub const MAX_ENCODED_BYTES: usize = MAX_REPLAY_DELTA_BYTES;

    pub fn new(contract: Option<ModelReplayContract>, items: Vec<ModelReplayItem>) -> Self {
        Self { contract, items }
    }

    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        replay_delta_is_valid(self.contract.as_ref(), &self.items, self.fits_capacity())
    }

    #[doc(hidden)]
    pub fn fits_capacity(&self) -> bool {
        Self::prospective_encoded_len(self.contract.as_ref(), self.items.iter())
            .is_some_and(|bytes| bytes <= MAX_REPLAY_DELTA_BYTES)
    }

    #[doc(hidden)]
    pub fn prospective_encoded_len<'a>(
        contract: Option<&ModelReplayContract>,
        items: impl Iterator<Item = &'a ModelReplayItem>,
    ) -> Option<usize> {
        let items = items.collect::<Vec<_>>();
        (items.len() <= MAX_REPLAY_ITEMS).then(|| encoded_prefix_len(contract, items.into_iter()))
    }

    pub fn replay_budget<'a>(
        contract: Option<&ModelReplayContract>,
        items: impl Iterator<Item = &'a ModelReplayItem>,
    ) -> Option<ModelReplayBudget> {
        let items = items.collect::<Vec<_>>();
        let encoded_prefix_bytes = (items.len() <= MAX_REPLAY_ITEMS)
            .then(|| encoded_prefix_len(contract, items.iter().copied()))?;
        (encoded_prefix_bytes <= MAX_REPLAY_DELTA_BYTES).then_some(ModelReplayBudget::from_prefix(
            encoded_prefix_bytes,
            items.len(),
        ))
    }

    #[doc(hidden)]
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_replay_delta(self.contract.as_ref(), &self.items, self.fits_capacity()).map(|_| ())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplay {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
    encoded_prefix_bytes: usize,
    known_calls: BTreeSet<String>,
    answered_calls: BTreeSet<String>,
}

impl ModelReplay {
    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    /// Builds one replay root recovered from a durable context checkpoint.
    ///
    /// Unlike an ordinary request-local delta, a checkpoint root may use the
    /// complete 64-MiB replay-prefix budget. Callers still have to preserve
    /// correlated semantic groups before flattening them into `items`.
    #[doc(hidden)]
    pub fn from_checkpoint(
        contract: ModelReplayContract,
        items: Vec<ModelReplayItem>,
    ) -> Result<Self, &'static str> {
        if !contract.is_valid() || items.is_empty() || items.len() > MAX_REPLAY_ITEMS {
            return Err("context checkpoint replay root is invalid or exceeds its item bound");
        }
        if items.iter().any(|item| !is_valid_item(item)) {
            return Err("context checkpoint replay root contains an invalid item");
        }
        let (known_calls, answered_calls) = validate_replay_items(&items)?;
        let encoded_prefix_bytes = encoded_prefix_len(Some(&contract), items.iter());
        if encoded_prefix_bytes > MAX_REPLAY_PREFIX_BYTES {
            return Err("context checkpoint replay root exceeds its byte bound");
        }
        Ok(Self {
            contract: Some(contract),
            items,
            encoded_prefix_bytes,
            known_calls,
            answered_calls,
        })
    }

    #[doc(hidden)]
    pub fn apply(&mut self, delta: &ModelReplayDelta) -> Result<(), &'static str> {
        self.apply_inner(delta, false)
    }

    /// Applies the first replay delta produced by a replacement binding.
    ///
    /// The replacement starts from an already reconstructed replay seed, but
    /// its first completed request must establish the new binding's exact
    /// system/tool contract instead of inheriting the source contract as its
    /// own chain declaration.
    #[doc(hidden)]
    pub fn apply_binding_replacement(
        &mut self,
        delta: &ModelReplayDelta,
    ) -> Result<(), &'static str> {
        self.apply_inner(delta, true)
    }

    fn apply_inner(
        &mut self,
        delta: &ModelReplayDelta,
        replace_contract: bool,
    ) -> Result<(), &'static str> {
        let (delta_calls, delta_answers) =
            validate_replay_delta(delta.contract(), delta.items(), delta.fits_capacity())?;
        if replace_contract {
            if self.contract.is_none() || delta.contract.is_none() {
                return Err("replacement binding first replay delta requires its new contract");
            }
        } else {
            match (self.contract.is_some(), delta.contract.is_some()) {
                (false, false) => return Err("first model replay delta requires its contract"),
                (true, true) => return Err("model replay contract was declared more than once"),
                (false, true) | (true, false) => {},
            }
        }
        if self.items.len().saturating_add(delta.items.len()) > MAX_REPLAY_ITEMS {
            return Err("model replay item limit exceeded");
        }
        if delta_calls
            .iter()
            .any(|call_id| self.known_calls.contains(call_id))
        {
            return Err("model replay contains a duplicate function call identity");
        }
        if delta_answers
            .iter()
            .any(|call_id| self.answered_calls.contains(call_id))
        {
            return Err("model replay contains a duplicate function call output");
        }
        let resulting_contract = if replace_contract {
            delta.contract.as_ref()
        } else {
            delta.contract.as_ref().or(self.contract.as_ref())
        };
        let encoded_prefix_bytes = encoded_prefix_len(
            resulting_contract,
            self.items.iter().chain(delta.items.iter()),
        );
        if encoded_prefix_bytes > MAX_REPLAY_PREFIX_BYTES {
            return Err("model replay prefix byte limit exceeded");
        }
        if replace_contract || self.contract.is_none() {
            let contract = delta
                .contract
                .as_ref()
                .expect("the selected replay-contract transition requires a contract");
            self.contract = Some(contract.clone());
        }
        self.items.extend(delta.items.iter().cloned());
        self.encoded_prefix_bytes = encoded_prefix_bytes;
        self.known_calls.extend(delta_calls);
        self.answered_calls.extend(delta_answers);
        Ok(())
    }
}
