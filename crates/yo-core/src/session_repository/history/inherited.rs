//! 상속된 source section을 자식 history로 조립하는 projection입니다.

use std::{collections::HashMap, sync::Arc};

use super::normalizer;
use crate::{
    JournalSequence, SessionId, TranscriptRecord,
    journal::codec::{ForkHistoryCoordinate, ForkSource, JournalRecord, RecoveredJournal},
};

/// 마지막으로 표시된 archival record와 독립적인 정확한 source 좌표입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InheritedHistorySource {
    Empty,
    Anchor {
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
    Checkpoint {
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
    InitialFork {
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
}

/// 한 원본 source Session의 identity와 순서를 보존하는 archival record 묶음입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InheritedHistorySection {
    source_session_id: SessionId,
    last_visible_journal_sequence: Option<JournalSequence>,
    records: Vec<TranscriptRecord>,
}

impl InheritedHistorySection {
    #[must_use]
    pub const fn source_session_id(&self) -> SessionId {
        self.source_session_id
    }

    /// 마지막으로 표시된 semantic 좌표이며 정확한 source capture 경계는 아닙니다.
    #[must_use]
    pub const fn last_visible_journal_sequence(&self) -> Option<JournalSequence> {
        self.last_visible_journal_sequence
    }

    #[must_use]
    pub fn records(&self) -> &[TranscriptRecord] {
        &self.records
    }
}

/// 자식 실행·request·usage와 분리된 검증된 자식 소유 archival history입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InheritedSessionHistory {
    parent_session_id: SessionId,
    source: InheritedHistorySource,
    sections: Arc<[InheritedHistorySection]>,
}

impl InheritedSessionHistory {
    #[must_use]
    pub const fn parent_session_id(&self) -> SessionId {
        self.parent_session_id
    }

    #[must_use]
    pub const fn source(&self) -> InheritedHistorySource {
        self.source
    }

    /// 최초 등장 순서로 정렬된 원본 source section입니다.
    #[must_use]
    pub fn sections(&self) -> &[InheritedHistorySection] {
        &self.sections
    }
}

pub(in crate::session_repository) fn project_inherited(
    recovered: &RecoveredJournal,
) -> Result<Option<Arc<InheritedSessionHistory>>, String> {
    let Some(seed) = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            JournalRecord::InitialForkSeed(seed) => Some(seed),
            _ => None,
        })
    else {
        return Ok(None);
    };
    let source = match seed.source() {
        ForkSource::Empty => InheritedHistorySource::Empty,
        ForkSource::Anchor(point) => InheritedHistorySource::Anchor {
            record_sequence: point.record_sequence(),
            journal_boundary: point.journal_boundary(),
        },
        ForkSource::Checkpoint(point) => InheritedHistorySource::Checkpoint {
            record_sequence: point.record_sequence(),
            journal_boundary: point.journal_boundary(),
        },
        ForkSource::InitialFork(point) => InheritedHistorySource::InitialFork {
            record_sequence: point.record_sequence(),
            journal_boundary: point.journal_boundary(),
        },
    };
    let mut source_indices = HashMap::new();
    let mut sections: Vec<InheritedHistorySection> = Vec::new();
    let mut records_by_source: Vec<Vec<&JournalRecord>> = Vec::new();
    for entry in seed.history() {
        let index = *source_indices
            .entry(entry.source_session_id())
            .or_insert_with(|| {
                let index = sections.len();
                sections.push(InheritedHistorySection {
                    source_session_id: entry.source_session_id(),
                    last_visible_journal_sequence: None,
                    records: Vec::new(),
                });
                records_by_source.push(Vec::new());
                index
            });
        if let ForkHistoryCoordinate::Journal { sequence } = entry.source_coordinate() {
            sections[index].last_visible_journal_sequence = Some(sequence);
        }
        records_by_source[index].push(entry.record().record());
    }
    for (section, records) in sections.iter_mut().zip(records_by_source) {
        section.records = normalizer::normalize_inherited(records)?;
    }
    Ok(Some(Arc::new(InheritedSessionHistory {
        parent_session_id: seed.parent_session_id(),
        source,
        sections: sections.into(),
    })))
}
