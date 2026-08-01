use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use super::super::{
    DurableRecord, DurableRecordKind, RepositoryEntry, RepositoryError, RepositorySequence,
};
use crate::{JournalSequence, SessionId};

const LEGACY_SCHEMA: &str = "yo.session-record/v1";
const PREVIOUS_SCHEMA: &str = "yo.session-record/v2";
const SCHEMA: &str = "yo.session-record/v3";
const CHECKSUM_SCHEMA: &str = "crc32c/v1";
const CHECKSUM_DOMAIN: &[u8] = b"yo.session-record.checksum/v1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireEntry {
    schema: String,
    session_id: WireSessionId,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum WireSessionId {
    Uuid(String),
    Legacy(u64),
}

enum WireSessionIdBytes {
    Legacy([u8; 8]),
    Uuid([u8; 16]),
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
        let session_id = WireSessionId::from_session(session_id);
        let checksum = checksum(
            SCHEMA,
            session_id.bytes(),
            sequence.get(),
            kind,
            journal_sequence,
            record.payload().as_bytes(),
        );
        Self {
            schema: SCHEMA.to_owned(),
            session_id,
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
        if self.schema != LEGACY_SCHEMA && self.schema != PREVIOUS_SCHEMA && self.schema != SCHEMA {
            return Err(corrupt(
                line,
                format!("unsupported schema {:?}", self.schema),
            ));
        }
        let actual_session = self.session_id.to_session(&self.schema, line)?;
        if actual_session != expected_session {
            return Err(corrupt(
                line,
                format!(
                    "expected Session {}, found {}",
                    expected_session, actual_session
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
            self.session_id.bytes(),
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
    session_id: WireSessionIdBytes,
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
    match session_id {
        WireSessionIdBytes::Legacy(bytes) => append_field(&mut preimage, &bytes),
        WireSessionIdBytes::Uuid(bytes) => append_field(&mut preimage, &bytes),
    }
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

impl WireSessionId {
    fn from_session(session_id: SessionId) -> Self {
        match session_id.as_uuid() {
            Some(uuid) => Self::Uuid(uuid.to_string()),
            None => Self::Legacy(
                session_id
                    .legacy_value()
                    .expect("a Session identity has one representation")
                    .get(),
            ),
        }
    }

    fn to_session(&self, schema: &str, line: usize) -> Result<SessionId, RepositoryError> {
        match (schema, self) {
            (SCHEMA, Self::Uuid(value)) => value.parse().map_err(|_| {
                corrupt(
                    line,
                    "current Session record identity is not a UUIDv7".to_owned(),
                )
            }),
            (LEGACY_SCHEMA | PREVIOUS_SCHEMA, Self::Legacy(value)) => NonZeroU64::new(*value)
                .map(SessionId::from_legacy)
                .ok_or_else(|| {
                    corrupt(line, "legacy Session identity must be non-zero".to_owned())
                }),
            (SCHEMA, Self::Legacy(_)) => Err(corrupt(
                line,
                "current Session record cannot carry a numeric identity".to_owned(),
            )),
            (LEGACY_SCHEMA | PREVIOUS_SCHEMA, Self::Uuid(_)) => Err(corrupt(
                line,
                "legacy Session record cannot carry a UUID identity".to_owned(),
            )),
            _ => Err(corrupt(line, format!("unsupported schema {schema:?}"))),
        }
    }

    fn bytes(&self) -> WireSessionIdBytes {
        match self {
            Self::Legacy(value) => WireSessionIdBytes::Legacy(value.to_be_bytes()),
            Self::Uuid(value) => {
                let uuid = uuid::Uuid::parse_str(value)
                    .expect("new records are constructed from a validated UUID");
                WireSessionIdBytes::Uuid(*uuid.as_bytes())
            },
        }
    }
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
