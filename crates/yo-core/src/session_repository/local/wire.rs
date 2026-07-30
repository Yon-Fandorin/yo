use serde::{Deserialize, Serialize};

use super::super::{
    DurableRecord, DurableRecordKind, RepositoryEntry, RepositoryError, RepositorySequence,
};
use crate::SessionId;

const SCHEMA: &str = "yo.session-record/v1";

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct WireEntry {
    schema: String,
    session_id: u64,
    sequence: u64,
    kind: WireRecordKind,
    payload: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireRecordKind {
    Incremental,
    Snapshot,
}

impl WireEntry {
    pub(super) fn from_record(
        session_id: SessionId,
        sequence: RepositorySequence,
        record: &DurableRecord,
    ) -> Self {
        let kind = match record.kind() {
            DurableRecordKind::Incremental => WireRecordKind::Incremental,
            DurableRecordKind::Snapshot => WireRecordKind::Snapshot,
        };
        Self {
            schema: SCHEMA.to_owned(),
            session_id: session_id.get().get(),
            sequence: sequence.get(),
            kind,
            payload: record.payload().to_owned(),
        }
    }

    pub(super) fn into_record(
        self,
        expected_session: SessionId,
        expected_sequence: u64,
        line: usize,
    ) -> Result<RepositoryEntry, RepositoryError> {
        if self.schema != SCHEMA {
            return Err(corrupt(
                line,
                format!("unsupported schema {:?}", self.schema),
            ));
        }
        if self.session_id != expected_session.get().get() {
            return Err(corrupt(
                line,
                format!(
                    "expected Session {}, found {}",
                    expected_session.get(),
                    self.session_id
                ),
            ));
        }
        if self.sequence != expected_sequence {
            return Err(corrupt(
                line,
                format!(
                    "expected sequence {expected_sequence}, found {}",
                    self.sequence
                ),
            ));
        }

        let record = match self.kind {
            WireRecordKind::Incremental => DurableRecord::incremental(self.payload),
            WireRecordKind::Snapshot => DurableRecord::snapshot(self.payload),
        };
        Ok(RepositoryEntry::new(
            RepositorySequence::new(self.sequence),
            record,
        ))
    }
}

fn corrupt(line: usize, reason: String) -> RepositoryError {
    RepositoryError::CorruptLog { line, reason }
}
