use serde::{Deserialize, Serialize};

use super::super::{
    DurableRecord, DurableRecordKind, RepositoryEntry, RepositoryError, RepositorySequence,
    SessionDiscovery, SessionRecordVersion,
};
use crate::{
    HostWorkspacePath, JournalSequence, SessionDescriptor, SessionId, SessionStartTime,
    WorkspaceHostId,
};

const SCHEMA: &str = "yo.session-record/v1";
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
    discovery: WireDiscovery,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<WireChecksum>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireDiscovery {
    descriptor: WireDescriptor,
    updated_unix_millis: u64,
    binding_epoch: Option<u64>,
    continuation_anchor_journal_sequence: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireDescriptor {
    session_id: WireSessionId,
    workspace_host_id: uuid::Uuid,
    workspace_path: WireWorkspacePath,
    started_unix_millis: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "encoding",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum WireWorkspacePath {
    Utf8(String),
    UnixBytes(Vec<u8>),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireChecksum {
    schema: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct WireSchemaProbe {
    schema: String,
    #[serde(flatten)]
    _ignored: std::collections::BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireRecordKind {
    Incremental,
    Snapshot,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(transparent)]
struct WireSessionId(uuid::Uuid);

impl WireRecordKind {
    const fn tag(self) -> u8 {
        match self {
            Self::Incremental => 1,
            Self::Snapshot => 2,
        }
    }
}

impl WireEntry {
    pub(super) fn decode(bytes: &[u8], line: usize) -> Result<Self, RepositoryError> {
        let probe: WireSchemaProbe =
            serde_json::from_slice(bytes).map_err(|error| RepositoryError::CorruptLog {
                line,
                reason: error.to_string(),
            })?;
        if probe.schema != SCHEMA {
            return Err(RepositoryError::UnsupportedSchema {
                schema: probe.schema,
            });
        }
        serde_json::from_slice(bytes).map_err(|error| RepositoryError::CorruptLog {
            line,
            reason: error.to_string(),
        })
    }

    pub(super) fn decode_tail(bytes: &[u8]) -> Result<Self, RepositoryError> {
        Self::decode(bytes, 0).map_err(tail_error)
    }

    pub(super) fn from_record(
        session_id: SessionId,
        sequence: RepositorySequence,
        record: &DurableRecord,
        updated_unix_millis: u64,
    ) -> Result<Self, RepositoryError> {
        let kind = match record.kind() {
            DurableRecordKind::Incremental => WireRecordKind::Incremental,
            DurableRecordKind::Snapshot => WireRecordKind::Snapshot,
        };
        let journal_sequence = record.journal_cutoff().map(JournalSequence::get);
        if journal_sequence == Some(0) {
            return Err(RepositoryError::Unavailable {
                message: "Journal cutoff sequence must be positive".to_owned(),
            });
        }
        let session_id = WireSessionId::from_session(session_id);
        let discovery = WireDiscovery::from_record(record, updated_unix_millis)?;
        if discovery.descriptor.session_id.bytes() != session_id.bytes() {
            return Err(RepositoryError::Unavailable {
                message: "Session discovery descriptor does not match the append target".to_owned(),
            });
        }
        let checksum = checksum(
            SCHEMA,
            session_id.bytes(),
            sequence.get(),
            kind,
            journal_sequence,
            record.payload().as_bytes(),
            &discovery,
        );
        Ok(Self {
            schema: SCHEMA.to_owned(),
            session_id,
            sequence: sequence.get(),
            kind,
            payload: record.payload().to_owned(),
            journal_sequence,
            discovery,
            checksum: Some(WireChecksum {
                schema: CHECKSUM_SCHEMA.to_owned(),
                value: format!("{checksum:08x}"),
            }),
        })
    }

    pub(super) fn into_record(
        self,
        expected_session: SessionId,
        expected_sequence: u64,
        line: usize,
    ) -> Result<DecodedEntry, RepositoryError> {
        if self.schema != SCHEMA {
            return Err(corrupt(
                line,
                format!("unsupported schema {:?}", self.schema),
            ));
        }
        let actual_session = self.session_id.to_session(line)?;
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
        if self.journal_sequence == Some(0) {
            return Err(corrupt(
                line,
                "Journal cutoff sequence must be positive".to_owned(),
            ));
        }
        self.validate_checksum(line)?;
        let discovery = self.discovery.into_domain(actual_session, line)?;

        let record = match self.kind {
            WireRecordKind::Incremental => DurableRecord::incremental(self.payload),
            WireRecordKind::Snapshot => DurableRecord::snapshot(self.payload),
        }
        .with_journal_cutoff(self.journal_sequence.map(JournalSequence::new));
        Ok(DecodedEntry {
            entry: RepositoryEntry::new(RepositorySequence::new(self.sequence), record),
            discovery,
        })
    }

    pub(super) fn into_tail(
        self,
        expected_session: SessionId,
    ) -> Result<(DecodedEntry, SessionRecordVersion), RepositoryError> {
        let sequence = self.sequence;
        if sequence == 0 {
            return Err(RepositoryError::CorruptTail {
                reason: "Session record sequence must be positive".to_owned(),
            });
        }
        self.into_record(expected_session, sequence, 0)
            .map_err(tail_error)
            .map(|entry| (entry, SessionRecordVersion::V1))
    }

    fn validate_checksum(&self, line: usize) -> Result<(), RepositoryError> {
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
            &self.discovery,
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
    session_id: [u8; 16],
    sequence: u64,
    kind: WireRecordKind,
    journal_sequence: Option<u64>,
    payload: &[u8],
    discovery: &WireDiscovery,
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
    append_field(&mut preimage, &session_id);
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
    discovery.append_checksum_fields(&mut preimage);
    crc32c(&preimage)
}

#[derive(Debug)]
pub(super) struct DecodedEntry {
    pub(super) entry: RepositoryEntry,
    pub(super) discovery: SessionDiscovery,
}

impl WireDiscovery {
    fn from_record(
        record: &DurableRecord,
        updated_unix_millis: u64,
    ) -> Result<Self, RepositoryError> {
        let discovery = record
            .discovery()
            .ok_or_else(|| RepositoryError::Unavailable {
                message: "a physical Session record requires discovery metadata".to_owned(),
            })?;
        let descriptor = discovery.descriptor();
        let path = descriptor.workspace_path().as_unix_bytes();
        let workspace_path = std::str::from_utf8(path).map_or_else(
            |_| WireWorkspacePath::UnixBytes(path.to_vec()),
            |path| WireWorkspacePath::Utf8(path.to_owned()),
        );
        let continuation_anchor_journal_sequence =
            discovery.continuation_anchor().map(JournalSequence::get);
        if continuation_anchor_journal_sequence == Some(0) {
            return Err(RepositoryError::Unavailable {
                message: "Continuation Anchor Journal sequence must be positive".to_owned(),
            });
        }
        Ok(Self {
            descriptor: WireDescriptor {
                session_id: WireSessionId::from_session(descriptor.session_id()),
                workspace_host_id: descriptor.workspace_host_id().as_uuid(),
                workspace_path,
                started_unix_millis: descriptor.started_at().unix_millis(),
            },
            updated_unix_millis,
            binding_epoch: discovery.binding_epoch(),
            continuation_anchor_journal_sequence,
        })
    }

    fn into_domain(
        self,
        expected_session: SessionId,
        line: usize,
    ) -> Result<SessionDiscovery, RepositoryError> {
        let descriptor_session = self.descriptor.session_id.to_session(line)?;
        if descriptor_session != expected_session {
            return Err(corrupt(
                line,
                "discovery descriptor belongs to another Session".to_owned(),
            ));
        }
        let host = WorkspaceHostId::from_uuid(self.descriptor.workspace_host_id)
            .map_err(|_| corrupt(line, "discovery Host identity is not a UUIDv4".to_owned()))?;
        let path = match self.descriptor.workspace_path {
            WireWorkspacePath::Utf8(path) => path.into_bytes(),
            WireWorkspacePath::UnixBytes(path) => path,
        };
        let path = HostWorkspacePath::from_unix_bytes(path)
            .map_err(|reason| corrupt(line, reason.to_owned()))?;
        let descriptor = SessionDescriptor::for_session(expected_session, host, path);
        if descriptor.started_at()
            != SessionStartTime::from_unix_millis(self.descriptor.started_unix_millis)
        {
            return Err(corrupt(
                line,
                "discovery start time does not match its UUIDv7 Session identity".to_owned(),
            ));
        }
        let continuation_anchor = match self.continuation_anchor_journal_sequence {
            Some(0) => {
                return Err(corrupt(
                    line,
                    "Continuation Anchor Journal sequence must be positive".to_owned(),
                ));
            },
            value => value.map(JournalSequence::new),
        };
        Ok(SessionDiscovery::new(
            descriptor,
            self.updated_unix_millis,
            self.binding_epoch,
            continuation_anchor,
        ))
    }

    fn append_checksum_fields(&self, preimage: &mut Vec<u8>) {
        append_field(preimage, self.descriptor.session_id.bytes().as_slice());
        append_field(preimage, self.descriptor.workspace_host_id.as_bytes());
        match &self.descriptor.workspace_path {
            WireWorkspacePath::Utf8(path) => {
                append_field(preimage, &[1]);
                append_field(preimage, path.as_bytes());
            },
            WireWorkspacePath::UnixBytes(path) => {
                append_field(preimage, &[2]);
                append_field(preimage, path);
            },
        }
        append_field(preimage, &self.descriptor.started_unix_millis.to_be_bytes());
        append_field(preimage, &self.updated_unix_millis.to_be_bytes());
        append_optional_u64(preimage, self.binding_epoch);
        append_optional_u64(preimage, self.continuation_anchor_journal_sequence);
    }
}

fn append_optional_u64(preimage: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            append_field(preimage, &[1]);
            append_field(preimage, &value.to_be_bytes());
        },
        None => append_field(preimage, &[0]),
    }
}

impl WireSessionId {
    fn from_session(session_id: SessionId) -> Self {
        Self(session_id.as_uuid())
    }

    fn to_session(&self, line: usize) -> Result<SessionId, RepositoryError> {
        SessionId::from_uuid(self.0)
            .map_err(|_| corrupt(line, "Session record identity is not a UUIDv7".to_owned()))
    }

    fn bytes(&self) -> [u8; 16] {
        *self.0.as_bytes()
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

fn tail_error(error: RepositoryError) -> RepositoryError {
    match error {
        RepositoryError::CorruptLog { reason, .. } => RepositoryError::CorruptTail { reason },
        error => error,
    }
}

#[cfg(test)]
mod tests {
    use super::{CHECKSUM_SCHEMA, WireChecksum, WireEntry, checksum, crc32c};
    use crate::session_repository::{DurableRecord, RecordDiscovery, RepositorySequence};

    fn refresh_checksum(wire: &mut WireEntry) {
        let value = checksum(
            &wire.schema,
            wire.session_id.bytes(),
            wire.sequence,
            wire.kind,
            wire.journal_sequence,
            wire.payload.as_bytes(),
            &wire.discovery,
        );
        wire.checksum = Some(WireChecksum {
            schema: CHECKSUM_SCHEMA.to_owned(),
            value: format!("{value:08x}"),
        });
    }

    // 표준 CRC32C 검사 벡터가 Castagnoli 다항식의 알려진 값과 일치해야 저장 레코드의
    // 무결성 검사가 다른 구현과 같은 바이트 의미를 사용한다고 신뢰할 수 있다.
    #[test]
    fn computes_the_standard_crc32c_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    // 고정 Session·descriptor·timestamp를 사용한 물리 v1의 CRC 답안을 코드 계산과
    // 독립된 상수로 남겨 discovery preimage 필드가 빠지거나 순서가 바뀌는 회귀를 잡습니다.
    #[test]
    fn physical_v1_checksum_has_a_stable_explicit_preimage() {
        let session_id = crate::fixture_session(12);
        let record = DurableRecord::snapshot("state")
            .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session_id)));
        let wire = WireEntry::from_record(
            session_id,
            RepositorySequence::new(1),
            &record,
            1_700_000_000_123,
        )
        .expect("the fixed physical v1 record encodes");
        let value = serde_json::to_value(wire).expect("wire record becomes JSON");

        assert_eq!(value["checksum"]["value"], "a52226f6");
    }

    // checksum까지 다시 맞춘 tail이라도 물리 순번 0은 첫 레코드가 1이라는 전체 replay
    // 규칙과 모순되므로 bounded discovery가 사용 가능한 Session으로 받아들이지 않습니다.
    #[test]
    fn tail_discovery_rejects_a_checksummed_zero_repository_sequence() {
        let session_id = crate::fixture_session(13);
        let record = DurableRecord::incremental("state")
            .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session_id)));
        let mut wire = WireEntry::from_record(
            session_id,
            RepositorySequence::new(1),
            &record,
            1_700_000_000_123,
        )
        .expect("the record encodes");
        wire.sequence = 0;
        refresh_checksum(&mut wire);

        let error = wire
            .into_tail(session_id)
            .expect_err("zero is not a valid physical sequence");

        assert!(error.to_string().contains("must be positive"));
    }

    // checksum에 포함된 anchor라도 Journal 순번 0은 실제 semantic record를 가리킬 수
    // 없으므로 picker가 이를 재개 가능한 근거로 오인하기 전에 wire 경계에서 거부합니다.
    #[test]
    fn tail_discovery_rejects_a_checksummed_zero_continuation_anchor() {
        let session_id = crate::fixture_session(14);
        let record = DurableRecord::incremental("state")
            .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session_id)));
        let mut wire = WireEntry::from_record(
            session_id,
            RepositorySequence::new(1),
            &record,
            1_700_000_000_123,
        )
        .expect("the record encodes");
        wire.discovery.continuation_anchor_journal_sequence = Some(0);
        refresh_checksum(&mut wire);

        let error = wire
            .into_tail(session_id)
            .expect_err("zero is not a valid Journal sequence");

        assert!(error.to_string().contains("must be positive"));
    }

    // checksum까지 유효한 semantic cutoff 0도 실제 Journal record를 가리킬 수 없으므로
    // 전체 replay와 tail discovery가 같은 양의 순번 규칙으로 이를 거부해야 합니다.
    #[test]
    fn tail_discovery_rejects_a_checksummed_zero_journal_cutoff() {
        let session_id = crate::fixture_session(15);
        let record = DurableRecord::incremental("state")
            .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session_id)));
        let mut wire = WireEntry::from_record(
            session_id,
            RepositorySequence::new(1),
            &record,
            1_700_000_000_123,
        )
        .expect("the record encodes");
        wire.journal_sequence = Some(0);
        refresh_checksum(&mut wire);

        let error = wire
            .into_tail(session_id)
            .expect_err("zero is not a valid Journal cutoff");

        assert!(error.to_string().contains("must be positive"));
    }

    // checksum과 값이 같은 payload를 두 번 적어도 닫힌 v1 shape의 중복 필드이므로,
    // schema 진단 probe가 JSON map으로 합친 뒤 정상 record로 받아들이지 않아야 합니다.
    #[test]
    fn supported_v1_rejects_a_duplicate_checksummed_payload_field() {
        let session_id = crate::fixture_session(16);
        let record = DurableRecord::incremental("state")
            .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session_id)));
        let wire = WireEntry::from_record(
            session_id,
            RepositorySequence::new(1),
            &record,
            1_700_000_000_123,
        )
        .expect("the record encodes");
        let encoded = serde_json::to_string(&wire)
            .expect("wire becomes JSON")
            .replace(
                "\"payload\":\"state\"",
                "\"payload\":\"state\",\"payload\":\"state\"",
            );

        let error = WireEntry::decode_tail(encoded.as_bytes())
            .expect_err("a duplicate closed-shape field is rejected");

        assert!(error.to_string().contains("duplicate field `payload`"));
    }
}
