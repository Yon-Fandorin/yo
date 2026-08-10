use super::{
    super::{
        StoredDiscoveryMismatch, StoredDiscoveryMismatchKind, StoredDiscoveryValidation,
        StoredSessionReadError, discovery_coordinates_mismatch, read_stored_session,
    },
    support::{MemoryReader, activity, finished, record_with_discovery, session, started},
};
use crate::{
    ActivityOutcome, JournalSequence,
    journal::codec::{
        JournalCommit, JournalRecord, MessageEnded, MessageOutcome, MessageStream, MessageTerminal,
        SequencedJournalRecord, encode,
    },
    session_repository::{DurableRecord, RecordDiscovery, RepositoryEntry, RepositorySequence},
};

// mismatch가 원인별 typed 좌표를 직접 소유하므로 서로 다른 두 sequence가 같은 숫자여도
// CLI 진단에서 repository 위치와 Journal 참조 위치를 구분할 수 있습니다.
#[test]
fn discovery_mismatch_diagnostics_preserve_typed_coordinates_and_causes() {
    let cases = [
        (
            StoredDiscoveryMismatch {
                repository_sequence: RepositorySequence::new(7),
                kind: StoredDiscoveryMismatchKind::Missing,
            },
            "metadata is missing at repository sequence 7",
        ),
        (
            StoredDiscoveryMismatch {
                repository_sequence: RepositorySequence::new(8),
                kind: StoredDiscoveryMismatchKind::Descriptor,
            },
            "descriptor disagrees with its semantic Journal at repository sequence 8",
        ),
        (
            StoredDiscoveryMismatch {
                repository_sequence: RepositorySequence::new(9),
                kind: StoredDiscoveryMismatchKind::BindingEpoch { claimed: 3 },
            },
            "binding epoch 3 at repository sequence 9 has no semantic Journal binding evidence",
        ),
        (
            StoredDiscoveryMismatch {
                repository_sequence: RepositorySequence::new(10),
                kind: StoredDiscoveryMismatchKind::ContinuationAnchor {
                    referenced: JournalSequence::new(4),
                },
            },
            "Continuation Anchor Journal sequence 4 at repository sequence 10 has no semantic Journal anchor evidence",
        ),
    ];

    for (mismatch, expected) in cases {
        assert_eq!(mismatch.to_string(), expected);
    }
}

// semantic recovery와 physical summary가 모두 값을 가지되 서로 다르면 “근거 없음”이
// 아니라 expected와 claimed를 함께 보존하는 불일치 원인으로 분류해야 합니다.
#[test]
fn distinguishes_coordinate_disagreement_from_missing_evidence() {
    assert_eq!(
        discovery_coordinates_mismatch(Some(1), Some(2), None, None),
        Some(StoredDiscoveryMismatchKind::BindingEpochDisagreement {
            expected: 1,
            claimed: 2,
        })
    );
    assert_eq!(
        discovery_coordinates_mismatch(
            Some(2),
            Some(2),
            Some(JournalSequence::new(8)),
            Some(JournalSequence::new(9)),
        ),
        Some(
            StoredDiscoveryMismatchKind::ContinuationAnchorDisagreement {
                expected: JournalSequence::new(8),
                claimed: JournalSequence::new(9),
            }
        )
    );
}

// semantic commit이 읽혀도 physical envelope에 필수 discovery summary가 없다면
// 해당 repository sequence를 typed mismatch로 식별합니다.
#[test]
fn identifies_the_envelope_that_is_missing_discovery() {
    let commit = JournalCommit::snapshot(vec![SequencedJournalRecord::new(
        JournalSequence::new(1),
        JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
    )]);
    let record = DurableRecord::snapshot(encode(&commit).unwrap())
        .with_journal_cutoff(commit.journal_cutoff());
    let reader = MemoryReader {
        entries: vec![RepositoryEntry::new(RepositorySequence::new(6), record)],
        missing: false,
    };

    let history = read_stored_session(&reader, session()).unwrap();

    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch {
            repository_sequence: RepositorySequence::new(6),
            kind: StoredDiscoveryMismatchKind::Missing,
        })
    );
}

// physical summary의 descriptor가 semantic descriptor와 다르면 backend 주장보다 먼저
// 그 envelope의 descriptor 불일치를 정확히 보고합니다.
#[test]
fn identifies_the_envelope_with_a_mismatched_descriptor() {
    let commit = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let reader = MemoryReader {
        entries: vec![RepositoryEntry::new(
            RepositorySequence::new(9),
            record_with_discovery(
                &commit,
                RecordDiscovery::new(crate::fixture_descriptor(crate::fixture_session(2))),
            ),
        )],
        missing: false,
    };

    let history = read_stored_session(&reader, session()).unwrap();

    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch {
            repository_sequence: RepositorySequence::new(9),
            kind: StoredDiscoveryMismatchKind::Descriptor,
        })
    );
}

// envelope의 snapshot 표기와 내부 commit 종류가 다르면 부분 투영하지 않고 정확한
// physical repository sequence가 포함된 경계 오류를 반환한다.
#[test]
fn rejects_an_envelope_kind_mismatch() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::new(
                    activity(),
                    MessageStream::Agent,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(4, ActivityOutcome::Completed),
    ]);
    let mismatched = DurableRecord::incremental(encode(&commit).unwrap())
        .with_journal_cutoff(commit.journal_cutoff())
        .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session())));
    let reader = MemoryReader {
        entries: vec![RepositoryEntry::new(RepositorySequence::new(7), mismatched)],
        missing: false,
    };

    let error = read_stored_session(&reader, session()).unwrap_err();

    assert!(error.to_string().contains("repository sequence 7"));
    assert!(error.to_string().contains("envelope does not match"));
}

// 파일 자체가 없는 경우와 파일은 있지만 complete envelope가 하나도 없는 경우는 복구
// 조치가 다르므로 direct read error에서도 각각 NotFound와 Incomplete로 구분합니다.
#[test]
fn distinguishes_missing_and_incomplete_physical_history() {
    let missing = MemoryReader {
        entries: Vec::new(),
        missing: true,
    };
    let incomplete = MemoryReader::default();

    assert!(matches!(
        read_stored_session(&missing, session()),
        Err(StoredSessionReadError::NotFound { .. })
    ));
    assert!(matches!(
        read_stored_session(&incomplete, session()),
        Err(StoredSessionReadError::Incomplete { .. })
    ));
}
