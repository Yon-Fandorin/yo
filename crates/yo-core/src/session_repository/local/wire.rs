use serde::{Deserialize, Serialize};

use super::super::{
    DurableRecord, DurableRecordKind, RepositoryEntry, RepositoryError, RepositorySequence,
};
use crate::{JournalSequence, SessionId};

const LEGACY_SCHEMA: &str = "yo.session-record/v1";
const SCHEMA: &str = "yo.session-record/v2";
const CHECKSUM_SCHEMA: &str = "crc32c/v1";
const CHECKSUM_DOMAIN: &[u8] = b"yo.session-record.checksum/v1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireEntry {
    schema: String,
    session_id: u64,
    sequence: u64,
    kind: WireRecordKind,
    payload: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    journal_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<WireChecksum>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireChecksum {
    schema: String,
    value: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireRecordKind {
    Incremental,
    Snapshot,
}

impl WireRecordKind {
    const fn tag(self) -> u8 {
        match self {
            Self::Incremental => 1,
            Self::Snapshot => 2,
        }
    }
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
        let journal_sequence = record.journal_cutoff().map(JournalSequence::get);
        let checksum = checksum(
            SCHEMA,
            session_id.get().get(),
            sequence.get(),
            kind,
            journal_sequence,
            record.payload().as_bytes(),
        );
        Self {
            schema: SCHEMA.to_owned(),
            session_id: session_id.get().get(),
            sequence: sequence.get(),
            kind,
            payload: record.payload().to_owned(),
            journal_sequence,
            checksum: Some(WireChecksum {
                schema: CHECKSUM_SCHEMA.to_owned(),
                value: format!("{checksum:08x}"),
            }),
        }
    }

    pub(super) fn into_record(
        self,
        expected_session: SessionId,
        expected_sequence: u64,
        line: usize,
    ) -> Result<RepositoryEntry, RepositoryError> {
        if self.schema != LEGACY_SCHEMA && self.schema != SCHEMA {
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
        self.validate_checksum(line)?;

        let record = match self.kind {
            WireRecordKind::Incremental => DurableRecord::incremental(self.payload),
            WireRecordKind::Snapshot => DurableRecord::snapshot(self.payload),
        }
        .with_journal_cutoff(self.journal_sequence.map(JournalSequence::new));
        Ok(RepositoryEntry::new(
            RepositorySequence::new(self.sequence),
            record,
        ))
    }

    fn validate_checksum(&self, line: usize) -> Result<(), RepositoryError> {
        if self.schema == LEGACY_SCHEMA {
            if self.checksum.is_some() || self.journal_sequence.is_some() {
                return Err(corrupt(
                    line,
                    "legacy records cannot carry v2 checksum fields".to_owned(),
                ));
            }
            return Ok(());
        }

        let checksum_record = self
            .checksum
            .as_ref()
            .ok_or_else(|| corrupt(line, "checksummed record has no checksum".to_owned()))?;
        if checksum_record.schema != CHECKSUM_SCHEMA {
            return Err(corrupt(
                line,
                format!("unsupported checksum schema {:?}", checksum_record.schema),
            ));
        }
        let expected = checksum(
            &self.schema,
            self.session_id,
            self.sequence,
            self.kind,
            self.journal_sequence,
            self.payload.as_bytes(),
        );
        let actual = u32::from_str_radix(&checksum_record.value, 16)
            .map_err(|_| corrupt(line, "checksum is not eight hexadecimal digits".to_owned()))?;
        if checksum_record.value.len() != 8 || actual != expected {
            return Err(corrupt(line, "CRC32C checksum mismatch".to_owned()));
        }
        Ok(())
    }
}

fn checksum(
    schema: &str,
    session_id: u64,
    sequence: u64,
    kind: WireRecordKind,
    journal_sequence: Option<u64>,
    payload: &[u8],
) -> u32 {
    let mut preimage = Vec::with_capacity(
        CHECKSUM_DOMAIN
            .len()
            .saturating_add(schema.len())
            .saturating_add(payload.len())
            .saturating_add(64),
    );
    append_field(&mut preimage, CHECKSUM_DOMAIN);
    append_field(&mut preimage, schema.as_bytes());
    append_field(&mut preimage, &session_id.to_be_bytes());
    append_field(&mut preimage, &sequence.to_be_bytes());
    append_field(&mut preimage, &[kind.tag()]);
    match journal_sequence {
        Some(sequence) => {
            append_field(&mut preimage, &[1]);
            append_field(&mut preimage, &sequence.to_be_bytes());
        },
        None => append_field(&mut preimage, &[0]),
    }
    append_field(&mut preimage, payload);
    crc32c(&preimage)
}

fn append_field(preimage: &mut Vec<u8>, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    preimage.extend_from_slice(&length.to_be_bytes());
    preimage.extend_from_slice(bytes);
}

fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82f6_3b78 & mask);
        }
    }
    !crc
}

fn corrupt(line: usize, reason: String) -> RepositoryError {
    RepositoryError::CorruptLog { line, reason }
}

#[cfg(test)]
mod tests {
    use super::crc32c;

    // 표준 CRC32C 검사 벡터가 Castagnoli 다항식의 알려진 값과 일치해야 저장 레코드의
    // 무결성 검사가 다른 구현과 같은 바이트 의미를 사용한다고 신뢰할 수 있다.
    #[test]
    fn computes_the_standard_crc32c_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }
}
