use super::super::super::{
    BackendExchangeObserved, ContextImageLoss, ExchangeDirection, ExchangeKind, InitialForkSeed,
    OperationId, VersionedIdentity,
};
use crate::{ContinuationStrategy, JournalSequence, ModelReplayItem, ReplayProfile, TurnId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RegisteredFork {
    pub(super) sequence: JournalSequence,
    pub(super) seed: Box<InitialForkSeed>,
    pub(super) owner_epoch: Option<u64>,
    pub(super) has_accepted_request: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReferenceTarget {
    Exchange(IndexedExchange),
    Anchor {
        epoch: u64,
        context_epoch: Option<u64>,
        journal_boundary: JournalSequence,
    },
    Checkpoint {
        epoch: u64,
        context_epoch: u64,
        binding_identity: VersionedIdentity,
        replay_profile: ReplayProfile,
        has_provider_private: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IndexedExchange {
    pub(super) epoch: u64,
    pub(super) operation_id: OperationId,
    pub(super) kind: ExchangeKind,
    pub(super) direction: ExchangeDirection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReplayDeltaSource {
    pub(super) epoch: u64,
    pub(super) context_epoch: u64,
    pub(super) turn_id: TurnId,
    pub(super) accepted_request_sequence: JournalSequence,
    pub(super) items: Vec<ModelReplayItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReplayGroup {
    pub(super) first_sequence: JournalSequence,
    pub(super) last_sequence: JournalSequence,
    pub(super) replay_delta_sequence: JournalSequence,
    pub(super) epoch: u64,
    pub(super) context_epoch: u64,
    pub(super) items: Vec<ModelReplayItem>,
    pub(super) fork_group_index: Option<usize>,
    pub(super) image_losses: Vec<ContextImageLoss>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplacementSource {
    InitialFork {
        sequence: JournalSequence,
        epoch: u64,
    },
    Anchor {
        sequence: JournalSequence,
        epoch: u64,
    },
    Checkpoint {
        sequence: JournalSequence,
        epoch: u64,
    },
}

impl From<&BackendExchangeObserved> for IndexedExchange {
    fn from(exchange: &BackendExchangeObserved) -> Self {
        Self {
            epoch: exchange.epoch(),
            operation_id: exchange.operation_id(),
            kind: exchange.kind(),
            direction: exchange.direction(),
        }
    }
}
