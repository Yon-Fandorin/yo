use super::{CHECKSUM_SCHEMA, WireChecksum, WireEntry, checksum, crc32c};
use crate::{
    JournalSequence, fixture_descriptor, fixture_session,
    session_repository::{
        ContinuationEligibility, DurableRecord, RecordDiscovery, RepositorySequence, StoredSession,
        StoredSessionSummary,
    },
};

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
    let session_id = fixture_session(12);
    let record = DurableRecord::snapshot("state")
        .with_discovery(RecordDiscovery::new(fixture_descriptor(session_id)));
    let wire = WireEntry::from_record(
        session_id,
        RepositorySequence::new(1),
        &record,
        1_700_000_000_123,
    )
    .expect("the fixed physical v1 record encodes");
    let value = serde_json::to_value(wire).expect("wire record becomes JSON");

    assert_eq!(value["checksum"]["value"], "a52226f6");
    assert!(
        value["discovery"]
            .get("initial_fork_seed_journal_sequence")
            .is_none()
    );
}

fn fork_discovery_wire() -> WireEntry {
    let session = fixture_session(12);
    let record = DurableRecord::snapshot("state").with_discovery(
        RecordDiscovery::new(fixture_descriptor(session))
            .with_initial_fork_seed(JournalSequence::new(7)),
    );
    WireEntry::from_record(
        session,
        RepositorySequence::new(1),
        &record,
        1_700_000_000_123,
    )
    .expect("fork discovery encodes")
}

// 기존 고정 CRC 뒤에 계약의 두 length-prefixed 필드만 추가한 독립 답안으로,
// fork hint가 기존 레코드의 checksum 의미를 바꾸지 않고 별도로 보호되는지 확인합니다.
#[test]
fn fork_discovery_extends_the_frozen_checksum_and_survives_both_reader_paths() {
    let encoded = serde_json::to_vec(&fork_discovery_wire()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["checksum"]["value"], "beda16a2");
    assert_eq!(value["discovery"]["initial_fork_seed_journal_sequence"], 7);

    let session = fixture_session(12);
    let replay = WireEntry::decode(&encoded, 1)
        .unwrap()
        .into_record(session, 1, 1)
        .unwrap();
    assert_eq!(
        replay.discovery.initial_fork_seed(),
        Some(JournalSequence::new(7))
    );
    assert_eq!(
        replay
            .entry
            .record()
            .discovery()
            .unwrap()
            .initial_fork_seed(),
        Some(JournalSequence::new(7)),
    );
    let (tail, version) = WireEntry::decode_tail(&encoded)
        .unwrap()
        .into_tail(session)
        .unwrap();
    let summary = StoredSession::Available(StoredSessionSummary::new(
        tail.entry.sequence(),
        version,
        tail.discovery,
    ));
    assert_eq!(
        summary.continuation_eligibility(),
        ContinuationEligibility::Eligible
    );
}

// hint를 바꾸거나 제거한 JSON은 정상 형식이어도 원래 checksum으로 전체 읽기와
// bounded tail 읽기를 통과해서는 안 됩니다.
#[test]
fn fork_discovery_rejects_changed_or_removed_hint_with_original_checksum() {
    let original = serde_json::to_value(fork_discovery_wire()).unwrap();
    for replacement in [None, Some(8)] {
        let mut value = original.clone();
        let discovery = value["discovery"].as_object_mut().unwrap();
        match replacement {
            Some(sequence) => {
                discovery.insert(
                    "initial_fork_seed_journal_sequence".to_owned(),
                    sequence.into(),
                );
            },
            None => {
                discovery.remove("initial_fork_seed_journal_sequence");
            },
        }
        let encoded = serde_json::to_vec(&value).unwrap();
        assert!(
            WireEntry::decode(&encoded, 1)
                .unwrap()
                .into_record(fixture_session(12), 1, 1)
                .is_err()
        );
        assert!(
            WireEntry::decode_tail(&encoded)
                .unwrap()
                .into_tail(fixture_session(12))
                .is_err()
        );
    }
}

// 생략만 legacy 형식이며 명시적 null·0·음수·실수·문자열은 양의 정수 hint가
// 아니므로 checksum 비교나 목록 표시 전에 거부해야 합니다.
#[test]
fn fork_discovery_rejects_noncanonical_sequence_values() {
    let original = serde_json::to_value(fork_discovery_wire()).unwrap();
    for invalid in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(1.0),
        serde_json::json!(1.5),
        serde_json::json!(true),
        serde_json::json!("7"),
    ] {
        let mut value = original.clone();
        value["discovery"]["initial_fork_seed_journal_sequence"] = invalid;
        let encoded = serde_json::to_vec(&value).unwrap();
        assert!(WireEntry::decode(&encoded, 1).is_err());
        assert!(WireEntry::decode_tail(&encoded).is_err());
    }
}

// checksum까지 다시 맞춘 tail이라도 물리 순번 0은 첫 레코드가 1이라는 전체 replay
// 규칙과 모순되므로 bounded discovery가 사용 가능한 Session으로 받아들이지 않습니다.
#[test]
fn tail_discovery_rejects_a_checksummed_zero_repository_sequence() {
    let session_id = fixture_session(13);
    let record = DurableRecord::incremental("state")
        .with_discovery(RecordDiscovery::new(fixture_descriptor(session_id)));
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
    let session_id = fixture_session(14);
    let record = DurableRecord::incremental("state")
        .with_discovery(RecordDiscovery::new(fixture_descriptor(session_id)));
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
    let session_id = fixture_session(15);
    let record = DurableRecord::incremental("state")
        .with_discovery(RecordDiscovery::new(fixture_descriptor(session_id)));
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
    let session_id = fixture_session(16);
    let record = DurableRecord::incremental("state")
        .with_discovery(RecordDiscovery::new(fixture_descriptor(session_id)));
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
