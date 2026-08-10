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
