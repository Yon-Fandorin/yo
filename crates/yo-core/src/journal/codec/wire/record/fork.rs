//! Closed fork grammar, reusing this record owner's validated payload codecs.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use super::{
    super::{
        JournalCodecError,
        command::WireCommand,
        correlation::{self, WireContinuationStrategy, WireVersionedIdentity},
        event::WireEvent,
        identity::{WireActivityRef, WireSessionId, session_id_from},
        message::{WireMessageEnded, WireMessageReset, WireMessageSegment},
    },
    WireModelReplayContract, WireModelReplayItem, WireRecord, decode_model_replay_contract,
    decode_model_replay_items, encode_model_replay_contract, encode_model_replay_item,
};
use crate::{
    BackendBindingEvidence, BackendIdentity, JournalSequence, SessionId,
    journal::codec::{
        ForkExactReplay, ForkGroup, ForkHistoryCoordinate, ForkHistoryEntry, ForkItemCoordinate,
        ForkItemOrigin, ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint,
        INITIAL_FORK_SEED_PROFILE, InitialForkSeed, JournalRecord, ReplaySequence,
        SequencedJournalRecord, VersionedIdentity,
    },
};

const HISTORY_LIMIT: usize = 16 * 1024 * 1024;

#[cfg(test)]
mod tests;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct WireForkRecord {
    journal_sequence: u64,
    profile: String,
    parent_session_id: WireSessionId,
    source: WireSource,
    seed: WireSeed,
    history: Vec<WireHistoryEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireSource {
    Empty(WireEmpty),
    Anchor(WireSourcePoint),
    Checkpoint(WireSourcePoint),
    InitialFork(WireSourcePoint),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireEmpty {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireSourcePoint {
    binding_epoch: u64,
    context_epoch: u64,
    record_sequence: u64,
    journal_boundary: u64,
    binding: WireBinding,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireBinding {
    backend_kind: String,
    backend_version: String,
    binding_identity: WireVersionedIdentity,
    model_identity: WireVersionedIdentity,
    session_locator: WireVersionedIdentity,
    continuation_strategy: WireContinuationStrategy,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireSeed {
    Empty(WireEmpty),
    ExactReplay {
        contract: WireModelReplayContract,
        items: Vec<WireModelReplayItem>,
        item_origins: Vec<WireOrigin>,
        groups: Vec<WireGroup>,
    },
    BackendNative {
        source_boundary_evidence: WireVersionedIdentity,
        candidate_binding: WireBinding,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireCoordinate {
    session_id: WireSessionId,
    binding_epoch: u64,
    context_epoch: u64,
    record_sequence: u64,
    item_index: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireOrigin {
    original: WireCoordinate,
    imported_from: WireCoordinate,
    source_binding: WireBinding,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireGroup {
    first_item: usize,
    end_item: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireHistoryEntry {
    source_session_id: WireSessionId,
    source_coordinate: WireHistoryCoordinate,
    record: WireArchivedRecord,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireHistoryCoordinate {
    Journal {
        sequence: u64,
    },
    Message {
        activity_id: WireActivityRef,
        part: WireMessagePart,
        ordinal: u64,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireMessagePart {
    Reset,
    Segment,
    Ended,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireArchivedRecord {
    CommandCommitted {
        journal_sequence: u64,
        command: WireCommand,
    },
    EventCommitted {
        journal_sequence: u64,
        event: WireEvent,
    },
    MessageReset {
        reset: WireMessageReset,
    },
    MessageSegment {
        segment: WireMessageSegment,
    },
    MessageEnded {
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            deserialize_with = "non_null_segment"
        )]
        final_segment: Option<WireMessageSegment>,
        ended: WireMessageEnded,
    },
}

fn non_null_segment<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<WireMessageSegment>, D::Error> {
    WireMessageSegment::deserialize(decoder).map(Some)
}

fn history_bytes(history: &[WireHistoryEntry]) -> Result<usize, JournalCodecError> {
    if history.len() > 4096 {
        return Err(JournalCodecError::new("fork history item limit exceeded"));
    }
    let mut counter = HistoryByteCounter { bytes: 0 };
    serde_json::to_writer(&mut counter, history).map_err(|_| {
        JournalCodecError::new("fork history encoding failed or exceeded its byte limit")
    })?;
    Ok(counter.bytes)
}

struct HistoryByteCounter {
    bytes: usize,
}

impl Write for HistoryByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let total = self
            .bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= HISTORY_LIMIT)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fork history byte limit exceeded",
                )
            })?;
        self.bytes = total;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn validate_child(
    child: SessionId,
    seed: &InitialForkSeed,
) -> Result<(), JournalCodecError> {
    let history = seed
        .history()
        .iter()
        .map(WireHistoryEntry::encode)
        .collect::<Result<Vec<_>, _>>()?;
    InitialForkSeed::new(
        child,
        seed.parent_session_id(),
        seed.source().clone(),
        seed.seed().clone(),
        seed.history().to_vec(),
        history_bytes(&history)?,
    )
    .map(|_| ())
}

pub(super) fn prepare_seed(
    child: SessionId,
    parent: SessionId,
    source: ForkSource,
    seed: ForkSeed,
    history: Vec<(SessionId, ForkHistoryCoordinate, &SequencedJournalRecord)>,
) -> Result<InitialForkSeed, JournalCodecError> {
    if history.len() > 4096 {
        return Err(JournalCodecError::new("fork history item limit exceeded"));
    }
    let limit_error =
        || JournalCodecError::new("fork history encoding failed or exceeded its byte limit");
    let mut counter = HistoryByteCounter { bytes: 0 };
    counter.write_all(b"[").map_err(|_| limit_error())?;
    let mut retained = Vec::with_capacity(history.len());
    for (session, coordinate, record) in history {
        if !retained.is_empty() {
            counter.write_all(b",").map_err(|_| limit_error())?;
            if counter.bytes >= HISTORY_LIMIT {
                return Err(limit_error());
            }
        }
        let entry = ForkHistoryEntry::new(session, coordinate, record.clone())?;
        let encoded = WireHistoryEntry::encode(&entry)?;
        serde_json::to_writer(&mut counter, &encoded).map_err(|_| limit_error())?;
        // 마지막 ']'의 한 byte까지 확보한 entry만 보존하고 다음 record를 복제한다.
        if counter.bytes >= HISTORY_LIMIT {
            return Err(limit_error());
        }
        retained.push(entry);
    }
    counter.write_all(b"]").map_err(|_| limit_error())?;
    InitialForkSeed::new(child, parent, source, seed, retained, counter.bytes)
}

impl WireForkRecord {
    pub(super) fn encode(
        sequence: JournalSequence,
        seed: &InitialForkSeed,
    ) -> Result<Self, JournalCodecError> {
        let history = seed
            .history()
            .iter()
            .map(WireHistoryEntry::encode)
            .collect::<Result<Vec<_>, _>>()?;
        history_bytes(&history)?;
        Ok(Self {
            journal_sequence: sequence.get(),
            profile: INITIAL_FORK_SEED_PROFILE.to_owned(),
            parent_session_id: seed.parent_session_id().into(),
            source: WireSource::encode(seed.source())?,
            seed: WireSeed::encode(seed.seed())?,
            history,
        })
    }
    pub(super) fn decode(
        self,
        child: SessionId,
    ) -> Result<(Option<JournalSequence>, JournalRecord), JournalCodecError> {
        if self.profile != INITIAL_FORK_SEED_PROFILE {
            return Err(JournalCodecError::new("unsupported fork seed profile"));
        }
        let sequence = correlation::sequence(self.journal_sequence, "fork seed sequence")?;
        let byte_count = history_bytes(&self.history)?;
        let history = self
            .history
            .into_iter()
            .enumerate()
            .map(|(index, entry)| entry.decode(index))
            .collect::<Result<Vec<_>, _>>()?;
        let seed = InitialForkSeed::new(
            child,
            session_id_from(self.parent_session_id, "fork parent")?,
            self.source.decode()?,
            self.seed.decode()?,
            history,
            byte_count,
        )?;
        Ok((
            Some(sequence),
            JournalRecord::InitialForkSeed(Box::new(seed)),
        ))
    }
}

impl WireBinding {
    fn encode(binding: &BackendBindingEvidence) -> Result<Self, JournalCodecError> {
        if !binding.is_valid() {
            return Err(JournalCodecError::new("invalid frozen fork binding"));
        }
        let identity = |value: &BackendIdentity| {
            correlation::encode_identity(&VersionedIdentity::new(value.schema(), value.value()))
        };
        Ok(Self {
            backend_kind: binding.backend_kind().into(),
            backend_version: binding.backend_version().into(),
            binding_identity: identity(binding.binding_identity())?,
            model_identity: identity(binding.model_identity())?,
            session_locator: identity(binding.session_locator())?,
            continuation_strategy: binding.continuation_strategy().into(),
        })
    }
    fn decode(self) -> Result<BackendBindingEvidence, JournalCodecError> {
        correlation::validate_ascii(&self.backend_kind, "fork backend kind")?;
        correlation::validate_value(&self.backend_version, "fork backend version")?;
        let identity = |wire| {
            correlation::decode_identity(wire)
                .map(|id| BackendIdentity::new(id.schema(), id.value()))
        };
        let binding = BackendBindingEvidence::new(
            self.backend_kind,
            self.backend_version,
            identity(self.binding_identity)?,
            identity(self.model_identity)?,
            identity(self.session_locator)?,
            self.continuation_strategy.try_into()?,
        );
        if !binding.is_valid() {
            return Err(JournalCodecError::new("invalid frozen fork binding"));
        }
        Ok(binding)
    }
}

impl WireSourcePoint {
    fn encode(point: &ForkSourcePoint) -> Result<Self, JournalCodecError> {
        Ok(Self {
            binding_epoch: point.binding_epoch(),
            context_epoch: point.context_epoch(),
            record_sequence: point.record_sequence().get(),
            journal_boundary: point.journal_boundary().get(),
            binding: WireBinding::encode(point.binding())?,
        })
    }
    fn decode(self) -> Result<ForkSourcePoint, JournalCodecError> {
        ForkSourcePoint::new(
            self.binding_epoch,
            self.context_epoch,
            correlation::sequence(self.record_sequence, "fork source record")?,
            correlation::sequence(self.journal_boundary, "fork source boundary")?,
            self.binding.decode()?,
        )
    }
}

impl WireSource {
    fn encode(source: &ForkSource) -> Result<Self, JournalCodecError> {
        Ok(match source {
            ForkSource::Empty => Self::Empty(WireEmpty {}),
            ForkSource::Anchor(p) => Self::Anchor(WireSourcePoint::encode(p)?),
            ForkSource::Checkpoint(p) => Self::Checkpoint(WireSourcePoint::encode(p)?),
            ForkSource::InitialFork(p) => Self::InitialFork(WireSourcePoint::encode(p)?),
        })
    }
    fn decode(self) -> Result<ForkSource, JournalCodecError> {
        Ok(match self {
            Self::Empty(_) => ForkSource::Empty,
            Self::Anchor(p) => ForkSource::Anchor(p.decode()?),
            Self::Checkpoint(p) => ForkSource::Checkpoint(p.decode()?),
            Self::InitialFork(p) => ForkSource::InitialFork(p.decode()?),
        })
    }
}

impl WireCoordinate {
    fn encode(c: &ForkItemCoordinate) -> Self {
        Self {
            session_id: c.session_id().into(),
            binding_epoch: c.binding_epoch(),
            context_epoch: c.context_epoch(),
            record_sequence: c.record_sequence().get(),
            item_index: c.item_index(),
        }
    }
    fn decode(self) -> Result<ForkItemCoordinate, JournalCodecError> {
        ForkItemCoordinate::new(
            session_id_from(self.session_id, "fork item Session")?,
            self.binding_epoch,
            self.context_epoch,
            correlation::sequence(self.record_sequence, "fork origin record")?,
            self.item_index,
        )
    }
}

impl WireSeed {
    fn encode(seed: &ForkSeed) -> Result<Self, JournalCodecError> {
        Ok(match seed {
            ForkSeed::Empty => Self::Empty(WireEmpty {}),
            ForkSeed::BackendNative {
                source_boundary_evidence,
                candidate_binding,
            } => Self::BackendNative {
                source_boundary_evidence: correlation::encode_identity(source_boundary_evidence)?,
                candidate_binding: WireBinding::encode(candidate_binding)?,
            },
            ForkSeed::ExactReplay(exact) => Self::ExactReplay {
                contract: encode_model_replay_contract(exact.contract()),
                items: exact
                    .items()
                    .iter()
                    .zip(exact.item_origins())
                    .map(|(item, origin)| {
                        encode_model_replay_item(item, origin.original().binding_epoch())
                    })
                    .collect(),
                item_origins: exact
                    .item_origins()
                    .iter()
                    .map(|origin| {
                        Ok(WireOrigin {
                            original: WireCoordinate::encode(origin.original()),
                            imported_from: WireCoordinate::encode(origin.imported_from()),
                            source_binding: WireBinding::encode(origin.source_binding())?,
                        })
                    })
                    .collect::<Result<_, JournalCodecError>>()?,
                groups: exact
                    .groups()
                    .iter()
                    .map(|g| WireGroup {
                        first_item: g.first_item(),
                        end_item: g.end_item(),
                    })
                    .collect(),
            },
        })
    }
    fn decode(self) -> Result<ForkSeed, JournalCodecError> {
        Ok(match self {
            Self::Empty(_) => ForkSeed::Empty,
            Self::BackendNative {
                source_boundary_evidence,
                candidate_binding,
            } => ForkSeed::BackendNative {
                source_boundary_evidence: correlation::decode_identity(source_boundary_evidence)?,
                candidate_binding: candidate_binding.decode()?,
            },
            Self::ExactReplay {
                contract,
                items,
                item_origins,
                groups,
            } => {
                if items.len() != item_origins.len() || items.len() > 4096 || groups.len() > 4096 {
                    return Err(JournalCodecError::new(
                        "fork replay origin cardinality or bounds invalid",
                    ));
                }
                let origins = item_origins
                    .into_iter()
                    .map(|o| {
                        ForkItemOrigin::new(
                            o.original.decode()?,
                            o.imported_from.decode()?,
                            o.source_binding.decode()?,
                        )
                    })
                    .collect::<Result<Vec<_>, JournalCodecError>>()?;
                let items = items
                    .into_iter()
                    .zip(&origins)
                    .map(|(item, origin)| {
                        let mut decoded = decode_model_replay_items(
                            vec![item],
                            origin.original().binding_epoch(),
                        )?;
                        decoded.pop().ok_or_else(|| {
                            JournalCodecError::new("fork item decoder returned no item")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let groups = groups
                    .into_iter()
                    .map(|g| ForkGroup::new(g.first_item, g.end_item))
                    .collect::<Result<Vec<_>, _>>()?;
                ForkSeed::ExactReplay(ForkExactReplay::new(
                    decode_model_replay_contract(contract),
                    items,
                    origins,
                    groups,
                )?)
            },
        })
    }
}

impl WireHistoryEntry {
    fn encode(entry: &ForkHistoryEntry) -> Result<Self, JournalCodecError> {
        let source_coordinate = match entry.source_coordinate() {
            ForkHistoryCoordinate::Journal { sequence } => WireHistoryCoordinate::Journal {
                sequence: sequence.get(),
            },
            ForkHistoryCoordinate::Message {
                activity,
                part,
                ordinal,
            } => WireHistoryCoordinate::Message {
                activity_id: activity.into(),
                part: match part {
                    ForkMessagePart::Reset => WireMessagePart::Reset,
                    ForkMessagePart::Segment => WireMessagePart::Segment,
                    ForkMessagePart::Ended => WireMessagePart::Ended,
                },
                ordinal,
            },
        };
        let record = match WireRecord::try_from(entry.record())? {
            WireRecord::CommandCommitted {
                journal_sequence,
                command,
            } => WireArchivedRecord::CommandCommitted {
                journal_sequence,
                command,
            },
            WireRecord::EventCommitted {
                journal_sequence,
                event,
            } => WireArchivedRecord::EventCommitted {
                journal_sequence,
                event,
            },
            WireRecord::MessageReset { reset } => WireArchivedRecord::MessageReset { reset },
            WireRecord::MessageSegment { segment } => {
                WireArchivedRecord::MessageSegment { segment }
            },
            WireRecord::MessageEnded {
                final_segment,
                ended,
            } => WireArchivedRecord::MessageEnded {
                final_segment,
                ended,
            },
            _ => return Err(JournalCodecError::new("non-archival fork history record")),
        };
        Ok(Self {
            source_session_id: entry.source_session_id().into(),
            source_coordinate,
            record,
        })
    }
    fn decode(self, index: usize) -> Result<ForkHistoryEntry, JournalCodecError> {
        let coordinate = match self.source_coordinate {
            WireHistoryCoordinate::Journal { sequence } => ForkHistoryCoordinate::Journal {
                sequence: correlation::sequence(sequence, "fork history sequence")?,
            },
            WireHistoryCoordinate::Message {
                activity_id,
                part,
                ordinal,
            } => ForkHistoryCoordinate::Message {
                activity: activity_id.try_into()?,
                part: match part {
                    WireMessagePart::Reset => ForkMessagePart::Reset,
                    WireMessagePart::Segment => ForkMessagePart::Segment,
                    WireMessagePart::Ended => ForkMessagePart::Ended,
                },
                ordinal,
            },
        };
        let record = match self.record {
            WireArchivedRecord::CommandCommitted {
                journal_sequence,
                command,
            } => WireRecord::CommandCommitted {
                journal_sequence,
                command,
            },
            WireArchivedRecord::EventCommitted {
                journal_sequence,
                event,
            } => WireRecord::EventCommitted {
                journal_sequence,
                event,
            },
            WireArchivedRecord::MessageReset { reset } => WireRecord::MessageReset { reset },
            WireArchivedRecord::MessageSegment { segment } => {
                WireRecord::MessageSegment { segment }
            },
            WireArchivedRecord::MessageEnded {
                final_segment,
                ended,
            } => WireRecord::MessageEnded {
                final_segment,
                ended,
            },
        };
        let (sequence, record) = record.try_into()?;
        ForkHistoryEntry::new(
            session_id_from(self.source_session_id, "fork history Session")?,
            coordinate,
            SequencedJournalRecord::decoded(
                ReplaySequence::new(index as u64 + 1),
                sequence,
                record,
            ),
        )
    }
}
