use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};

use super::super::{
    JournalCodecError, JournalRecord, MessageSegment, MessageStream, SequencedJournalRecord,
    VersionedIdentity,
};
use crate::{
    ActivityRef, AgentCommand, BackendBindingEvidence, BackendIdentity, ContinuationStrategy,
    JournalSequence, ModelReplay, ModelReplayContract, ModelReplayItem, SessionId,
    backend::validate_provider_private_replay_sequence, provider_private_schema,
};

pub(crate) const INITIAL_FORK_SEED_PROFILE: &str = "yo.session-fork-seed/v1";
const MAX_ITEMS: usize = 4096;
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;

fn invalid(detail: &'static str) -> JournalCodecError {
    JournalCodecError::new(detail)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ForkSourcePoint {
    binding_epoch: u64,
    context_epoch: u64,
    record_sequence: JournalSequence,
    journal_boundary: JournalSequence,
    binding: BackendBindingEvidence,
}

impl ForkSourcePoint {
    pub(crate) fn new(
        binding_epoch: u64,
        context_epoch: u64,
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
        binding: BackendBindingEvidence,
    ) -> Result<Self, JournalCodecError> {
        if binding_epoch == 0
            || context_epoch == 0
            || record_sequence.get() == 0
            || journal_boundary.get() == 0
            || !binding.is_valid()
        {
            return Err(invalid(
                "fork source has invalid epochs, sequences, or binding",
            ));
        }
        Ok(Self {
            binding_epoch,
            context_epoch,
            record_sequence,
            journal_boundary,
            binding,
        })
    }
    pub(crate) const fn binding_epoch(&self) -> u64 {
        self.binding_epoch
    }
    pub(crate) const fn context_epoch(&self) -> u64 {
        self.context_epoch
    }
    pub(crate) const fn record_sequence(&self) -> JournalSequence {
        self.record_sequence
    }
    pub(crate) const fn journal_boundary(&self) -> JournalSequence {
        self.journal_boundary
    }
    pub(crate) const fn binding(&self) -> &BackendBindingEvidence {
        &self.binding
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ForkSource {
    Empty,
    Anchor(ForkSourcePoint),
    Checkpoint(ForkSourcePoint),
    InitialFork(ForkSourcePoint),
}

impl ForkSource {
    pub(crate) const fn point(&self) -> Option<&ForkSourcePoint> {
        match self {
            Self::Empty => None,
            Self::Anchor(p) | Self::Checkpoint(p) | Self::InitialFork(p) => Some(p),
        }
    }
    fn validate(&self) -> Result<(), JournalCodecError> {
        let valid = match self {
            Self::Empty => true,
            Self::Anchor(p) => p.journal_boundary < p.record_sequence,
            Self::Checkpoint(p) => p.journal_boundary == p.record_sequence,
            Self::InitialFork(p) => p.record_sequence < p.journal_boundary,
        };
        if valid {
            Ok(())
        } else {
            Err(invalid(
                "fork source boundary does not match its source kind",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ForkItemCoordinate {
    session_id: SessionId,
    binding_epoch: u64,
    context_epoch: u64,
    record_sequence: JournalSequence,
    item_index: usize,
}

impl Hash for ForkItemCoordinate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.session_id.hash(state);
        self.binding_epoch.hash(state);
        self.context_epoch.hash(state);
        self.record_sequence.get().hash(state);
        self.item_index.hash(state);
    }
}

impl ForkItemCoordinate {
    pub(crate) fn new(
        session_id: SessionId,
        binding_epoch: u64,
        context_epoch: u64,
        record_sequence: JournalSequence,
        item_index: usize,
    ) -> Result<Self, JournalCodecError> {
        if binding_epoch == 0
            || context_epoch == 0
            || record_sequence.get() == 0
            || item_index >= MAX_ITEMS
        {
            return Err(invalid(
                "fork item coordinate is outside its bounded identity domain",
            ));
        }
        Ok(Self {
            session_id,
            binding_epoch,
            context_epoch,
            record_sequence,
            item_index,
        })
    }
    pub(crate) const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub(crate) const fn binding_epoch(&self) -> u64 {
        self.binding_epoch
    }
    pub(crate) const fn context_epoch(&self) -> u64 {
        self.context_epoch
    }
    pub(crate) const fn record_sequence(&self) -> JournalSequence {
        self.record_sequence
    }
    pub(crate) const fn item_index(&self) -> usize {
        self.item_index
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ForkItemOrigin {
    original: ForkItemCoordinate,
    imported_from: ForkItemCoordinate,
    source_binding: BackendBindingEvidence,
}

impl ForkItemOrigin {
    pub(crate) fn new(
        original: ForkItemCoordinate,
        imported_from: ForkItemCoordinate,
        source_binding: BackendBindingEvidence,
    ) -> Result<Self, JournalCodecError> {
        if !source_binding.is_valid() {
            return Err(invalid("fork item source binding is invalid"));
        }
        Ok(Self {
            original,
            imported_from,
            source_binding,
        })
    }
    pub(crate) const fn original(&self) -> &ForkItemCoordinate {
        &self.original
    }
    pub(crate) const fn imported_from(&self) -> &ForkItemCoordinate {
        &self.imported_from
    }
    pub(crate) const fn source_binding(&self) -> &BackendBindingEvidence {
        &self.source_binding
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ForkGroup {
    first_item: usize,
    end_item: usize,
}

impl ForkGroup {
    pub(crate) fn new(first_item: usize, end_item: usize) -> Result<Self, JournalCodecError> {
        if first_item >= end_item || end_item > MAX_ITEMS {
            return Err(invalid("fork group is empty or exceeds the item bound"));
        }
        Ok(Self {
            first_item,
            end_item,
        })
    }
    pub(crate) const fn first_item(&self) -> usize {
        self.first_item
    }
    pub(crate) const fn end_item(&self) -> usize {
        self.end_item
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ForkExactReplay {
    contract: ModelReplayContract,
    items: Vec<ModelReplayItem>,
    item_origins: Vec<ForkItemOrigin>,
    groups: Vec<ForkGroup>,
}

impl ForkExactReplay {
    pub(crate) fn new(
        contract: ModelReplayContract,
        items: Vec<ModelReplayItem>,
        item_origins: Vec<ForkItemOrigin>,
        groups: Vec<ForkGroup>,
    ) -> Result<Self, JournalCodecError> {
        if items.is_empty()
            || items.len() > MAX_ITEMS
            || items.len() != item_origins.len()
            || groups.is_empty()
            || groups.len() > MAX_ITEMS
        {
            return Err(invalid(
                "fork replay items, origins, or groups have invalid cardinality",
            ));
        }
        ModelReplay::from_checkpoint(contract.clone(), items.clone())
            .map_err(JournalCodecError::new)?;
        let mut end = 0;
        for group in &groups {
            if group.first_item != end || group.end_item > items.len() {
                return Err(invalid(
                    "fork groups must be a gap-free ordered item partition",
                ));
            }
            // 각 group을 따로 검증해 tool/private 연결을 잘라 보존하지 못하게 합니다.
            ModelReplay::from_checkpoint(
                contract.clone(),
                items[group.first_item..group.end_item].to_vec(),
            )
            .map_err(JournalCodecError::new)?;
            end = group.end_item;
        }
        if end != items.len() {
            return Err(invalid("fork groups do not cover all items"));
        }
        Ok(Self {
            contract,
            items,
            item_origins,
            groups,
        })
    }
    pub(crate) const fn contract(&self) -> &ModelReplayContract {
        &self.contract
    }
    pub(crate) fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }
    pub(crate) fn item_origins(&self) -> &[ForkItemOrigin] {
        &self.item_origins
    }
    pub(crate) fn groups(&self) -> &[ForkGroup] {
        &self.groups
    }

    fn validate_import(
        &self,
        child: SessionId,
        parent: SessionId,
        source: &ForkSourcePoint,
    ) -> Result<(), JournalCodecError> {
        let ContinuationStrategy::ExactReplay { replay_profile, .. } =
            source.binding.continuation_strategy()
        else {
            return Err(invalid(
                "exact fork seed requires an exact source replay binding",
            ));
        };
        if let Some(schema) = provider_private_schema(replay_profile) {
            validate_provider_private_replay_sequence(&self.items, schema)
                .map_err(JournalCodecError::new)?;
        }
        for (item, origin) in self.items.iter().zip(&self.item_origins) {
            if origin.original.session_id == child
                || origin.imported_from.session_id != parent
                || origin.imported_from.record_sequence > source.journal_boundary
            {
                return Err(invalid(
                    "fork item origin does not belong to the captured parent boundary",
                ));
            }
            if let ModelReplayItem::ProviderPrivateAssistant { envelope } = item
                && (provider_private_schema(replay_profile) != Some(envelope.schema())
                    || !same_exact_identity(&origin.source_binding, &source.binding))
            {
                return Err(invalid(
                    "fork private item is incompatible with its exact source binding",
                ));
            }
        }
        Ok(())
    }
}

fn same_exact_identity(left: &BackendBindingEvidence, right: &BackendBindingEvidence) -> bool {
    same_host_model_identity(left, right)
        && left.continuation_strategy() == right.continuation_strategy()
}

fn same_host_model_identity(left: &BackendBindingEvidence, right: &BackendBindingEvidence) -> bool {
    left.backend_kind() == right.backend_kind()
        && left.binding_identity() == right.binding_identity()
        && left.model_identity() == right.model_identity()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ForkSeed {
    Empty,
    ExactReplay(ForkExactReplay),
    BackendNative {
        source_boundary_evidence: VersionedIdentity,
        candidate_binding: BackendBindingEvidence,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ForkMessagePart {
    Reset,
    Segment,
    Ended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ForkHistoryCoordinate {
    Journal {
        sequence: JournalSequence,
    },
    Message {
        activity: ActivityRef,
        part: ForkMessagePart,
        ordinal: u64,
    },
}

impl Hash for ForkHistoryCoordinate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Journal { sequence } => sequence.get().hash(state),
            Self::Message {
                activity,
                part,
                ordinal,
            } => {
                activity.hash(state);
                part.hash(state);
                ordinal.hash(state);
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ForkHistoryEntry {
    source_session_id: SessionId,
    source_coordinate: ForkHistoryCoordinate,
    record: SequencedJournalRecord,
}

impl ForkHistoryEntry {
    pub(crate) fn new(
        source_session_id: SessionId,
        source_coordinate: ForkHistoryCoordinate,
        record: SequencedJournalRecord,
    ) -> Result<Self, JournalCodecError> {
        if record.record().session_id() != Some(source_session_id) {
            return Err(invalid(
                "fork history record does not match its qualified Session",
            ));
        }
        match (&source_coordinate, record.record()) {
            (
                ForkHistoryCoordinate::Journal { sequence },
                JournalRecord::CommandCommitted(command),
            ) => {
                if matches!(
                    command.command(),
                    AgentCommand::CreateSession { .. } | AgentCommand::CompactContext { .. }
                ) || sequence.get() == 0
                    || record.journal_sequence() != Some(*sequence)
                {
                    return Err(invalid(
                        "fork history command or Journal coordinate is forbidden",
                    ));
                }
            },
            (ForkHistoryCoordinate::Journal { sequence }, JournalRecord::EventCommitted(_)) => {
                if sequence.get() == 0 || record.journal_sequence() != Some(*sequence) {
                    return Err(invalid(
                        "fork history event Journal coordinate does not match",
                    ));
                }
            },
            (ForkHistoryCoordinate::Message { activity, part, .. }, archived) => {
                let observed = match archived {
                    JournalRecord::MessageReset(r) => (r.activity(), ForkMessagePart::Reset),
                    JournalRecord::MessageSegment(r) => (r.activity(), ForkMessagePart::Segment),
                    JournalRecord::MessageEnded(r) => {
                        (r.ended().activity(), ForkMessagePart::Ended)
                    },
                    _ => {
                        return Err(invalid(
                            "fork history message coordinate has a forbidden record",
                        ));
                    },
                };
                if activity.session_id() != source_session_id
                    || observed != (*activity, *part)
                    || record.journal_sequence().is_some()
                {
                    return Err(invalid(
                        "fork history message coordinate does not match its record",
                    ));
                }
            },
            _ => return Err(invalid("fork history contains a non-archival record")),
        }
        Ok(Self {
            source_session_id,
            source_coordinate,
            record: SequencedJournalRecord::decoded(
                super::super::ReplaySequence::new(match source_coordinate {
                    ForkHistoryCoordinate::Journal { sequence } => sequence.get(),
                    ForkHistoryCoordinate::Message { ordinal, .. } => ordinal
                        .checked_add(1)
                        .ok_or_else(|| invalid("fork archival message ordinal overflow"))?,
                }),
                record.journal_sequence(),
                record.record().clone(),
            ),
        })
    }
    pub(crate) const fn source_session_id(&self) -> SessionId {
        self.source_session_id
    }
    pub(crate) const fn source_coordinate(&self) -> ForkHistoryCoordinate {
        self.source_coordinate
    }
    pub(crate) const fn record(&self) -> &SequencedJournalRecord {
        &self.record
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InitialForkSeed {
    parent_session_id: SessionId,
    source: ForkSource,
    seed: ForkSeed,
    history: Vec<ForkHistoryEntry>,
}

impl InitialForkSeed {
    // wire 소유자가 배열 전체를 인코딩해 측정한 크기를 전달해야 합니다.
    pub(crate) fn new(
        child_session_id: SessionId,
        parent_session_id: SessionId,
        source: ForkSource,
        seed: ForkSeed,
        history: Vec<ForkHistoryEntry>,
        encoded_history_bytes: usize,
    ) -> Result<Self, JournalCodecError> {
        if child_session_id == parent_session_id
            || history.len() > MAX_ITEMS
            || !(2..=MAX_HISTORY_BYTES).contains(&encoded_history_bytes)
        {
            return Err(invalid(
                "fork seed identity or archival history bound is invalid",
            ));
        }
        let result = Self {
            parent_session_id,
            source,
            seed,
            history,
        };
        result.validate_child(child_session_id)?;
        Ok(result)
    }

    pub(crate) fn validate_child(
        &self,
        child_session_id: SessionId,
    ) -> Result<(), JournalCodecError> {
        if child_session_id == self.parent_session_id {
            return Err(invalid("fork child must differ from its parent"));
        }
        let parent_session_id = self.parent_session_id;
        let source = &self.source;
        let seed = &self.seed;
        let history = &self.history;
        source.validate()?;
        match (source, seed) {
            (ForkSource::Empty, ForkSeed::Empty) if history.is_empty() => {},
            (_, ForkSeed::ExactReplay(exact)) if source.point().is_some() => {
                exact.validate_import(
                    child_session_id,
                    parent_session_id,
                    source.point().expect("source checked"),
                )?;
            },
            (
                _,
                ForkSeed::BackendNative {
                    source_boundary_evidence,
                    candidate_binding,
                },
            ) if source.point().is_some() => {
                let original = &source.point().expect("source checked").binding;
                if !BackendIdentity::new(
                    source_boundary_evidence.schema(),
                    source_boundary_evidence.value(),
                )
                .is_valid()
                    || !candidate_binding.is_valid()
                    || !same_host_model_identity(original, candidate_binding)
                    || candidate_binding.continuation_strategy()
                        != ContinuationStrategy::BackendManagedState
                    || original.session_locator() == candidate_binding.session_locator()
                {
                    return Err(invalid(
                        "native fork has invalid boundary evidence or candidate identity",
                    ));
                }
                // 알려진 schema와 정확한 source 경계 증명은 host/recovery가 별도로 검증합니다.
            },
            _ => return Err(invalid("fork source, seed, and empty history do not agree")),
        }
        validate_history(history, child_session_id, parent_session_id, source.point())?;
        Ok(())
    }
    pub(crate) const fn parent_session_id(&self) -> SessionId {
        self.parent_session_id
    }
    pub(crate) const fn source(&self) -> &ForkSource {
        &self.source
    }
    pub(crate) const fn seed(&self) -> &ForkSeed {
        &self.seed
    }
    pub(crate) fn history(&self) -> &[ForkHistoryEntry] {
        &self.history
    }
}

#[derive(Default)]
struct ArchivedMessage {
    revision: Option<u64>,
    stream: Option<MessageStream>,
    segments: u64,
    bytes: u64,
    ended: bool,
}

impl ArchivedMessage {
    fn segment(&mut self, segment: &MessageSegment) -> Result<(), JournalCodecError> {
        if self
            .revision
            .is_some_and(|r| r.checked_add(1) == Some(segment.revision()))
        {
            if segment.index() != 1 {
                return Err(invalid("fork replacement segment must start at index one"));
            }
            self.revision = Some(segment.revision());
            self.segments = 0;
            self.bytes = 0;
        }
        self.owner(segment.revision(), segment.stream())?;
        if Some(segment.index()) != self.segments.checked_add(1)
            || segment.text().is_empty()
            || segment.text().len() > segment.stream().segment_limit()
        {
            return Err(invalid(
                "fork archival message segment order or size is invalid",
            ));
        }
        self.segments = self
            .segments
            .checked_add(1)
            .ok_or_else(|| invalid("fork message count overflow"))?;
        self.bytes = self
            .bytes
            .checked_add(segment.text().len() as u64)
            .ok_or_else(|| invalid("fork message size overflow"))?;
        Ok(())
    }
    fn owner(&mut self, revision: u64, stream: MessageStream) -> Result<(), JournalCodecError> {
        if self.ended
            || revision == 0
            || self.revision.is_some_and(|r| r != revision)
            || self.stream.is_some_and(|s| s != stream)
        {
            return Err(invalid(
                "fork archival message owner or terminal order is invalid",
            ));
        }
        self.revision = Some(revision);
        self.stream = Some(stream);
        Ok(())
    }
}

fn validate_history(
    history: &[ForkHistoryEntry],
    child: SessionId,
    parent: SessionId,
    source: Option<&ForkSourcePoint>,
) -> Result<(), JournalCodecError> {
    let mut coordinates = HashSet::new();
    let mut sequences = HashMap::new();
    let mut ordinals = HashMap::new();
    let mut messages: HashMap<ActivityRef, ArchivedMessage> = HashMap::new();
    for entry in history {
        if entry.source_session_id == child
            || !coordinates.insert((entry.source_session_id, entry.source_coordinate))
        {
            return Err(invalid(
                "fork history has a child identity or duplicate source coordinate",
            ));
        }
        match entry.source_coordinate {
            ForkHistoryCoordinate::Journal { sequence } => {
                if sequences
                    .insert(entry.source_session_id, sequence)
                    .is_some_and(|previous| previous >= sequence)
                    || (entry.source_session_id == parent
                        && source.is_some_and(|s| sequence > s.journal_boundary))
                {
                    return Err(invalid(
                        "fork history crosses its source boundary or sequence order",
                    ));
                }
            },
            ForkHistoryCoordinate::Message {
                activity,
                part,
                ordinal,
            } => {
                let next = ordinals.entry((activity, part)).or_insert(0u64);
                if ordinal != *next {
                    return Err(invalid("fork history message ordinal is not contiguous"));
                }
                *next = next
                    .checked_add(1)
                    .ok_or_else(|| invalid("fork message ordinal overflow"))?;
                let state = messages.entry(activity).or_default();
                match entry.record.record() {
                    JournalRecord::MessageReset(reset) => {
                        if state.ended
                            || reset.revision() == 0
                            || state
                                .revision
                                .is_some_and(|r| r.checked_add(1) != Some(reset.revision()))
                            || state.stream.is_some_and(|s| s != reset.stream())
                        {
                            return Err(invalid("fork archival message reset is invalid"));
                        }
                        *state = ArchivedMessage {
                            revision: Some(reset.revision()),
                            stream: Some(reset.stream()),
                            ..ArchivedMessage::default()
                        };
                    },
                    JournalRecord::MessageSegment(segment) => state.segment(segment)?,
                    JournalRecord::MessageEnded(terminal) => {
                        if let Some(segment) = terminal.final_segment() {
                            if segment.activity() != activity {
                                return Err(invalid("fork terminal segment activity mismatch"));
                            }
                            state.segment(segment)?;
                        }
                        let ended = terminal.ended();
                        if terminal.final_segment().is_none()
                            && ended.segment_count() == 0
                            && ended.utf8_bytes() == 0
                            && state
                                .revision
                                .is_some_and(|r| r.checked_add(1) == Some(ended.revision()))
                        {
                            state.revision = Some(ended.revision());
                            state.segments = 0;
                            state.bytes = 0;
                        }
                        state.owner(ended.revision(), ended.stream())?;
                        if ended.segment_count() != state.segments
                            || ended.utf8_bytes() != state.bytes
                        {
                            return Err(invalid(
                                "fork archival message terminal totals do not match",
                            ));
                        }
                        state.ended = true;
                    },
                    _ => {
                        return Err(invalid(
                            "fork history message coordinate lost its typed record",
                        ));
                    },
                }
            },
        }
    }
    if messages.values().any(|state| !state.ended) {
        return Err(invalid(
            "fork archival history contains an unterminated message",
        ));
    }
    Ok(())
}
