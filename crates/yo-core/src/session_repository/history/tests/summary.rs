use super::{
    super::{
        StoredDiscoveryMismatch, StoredDiscoveryMismatchKind, StoredDiscoveryValidation,
        read_stored_session,
    },
    support::{MemoryReader, record_with_discovery, session, started},
};
use crate::{
    AgentEvent, JournalSequence, TranscriptRecord,
    journal::codec::{
        BackendBindingOpened, BindingTransition, CacheState, JournalCommit, JournalRecord,
        ReplaySequence, SequencedJournalRecord, TransitionMode, VersionedIdentity,
    },
    session_repository::{
        DurableRecord, RecordDiscovery, RepositoryEntry, RepositorySequence,
        StoredRequestTraceRecord,
    },
};

fn record(commit: &JournalCommit) -> DurableRecord {
    record_with_discovery(
        commit,
        RecordDiscovery::new(crate::fixture_descriptor(session())),
    )
}

fn semantic_record(
    replay_sequence: u64,
    journal_sequence: u64,
    record: JournalRecord,
) -> SequencedJournalRecord {
    SequencedJournalRecord::with_journal_sequence(
        ReplaySequence::new(replay_sequence),
        JournalSequence::new(journal_sequence),
        record,
    )
}

fn identity(name: &str) -> VersionedIdentity {
    VersionedIdentity::new(format!("yo.test.{name}/v1"), format!("{name}:value"))
}

// semantic binding record가 epoch 1을 열고 physical discovery도 같은 값을 주장하면
// history reader는 correlation record를 Transcript에 노출하지 않으면서 summary를 검증합니다.
#[test]
fn accepts_a_discovery_epoch_derived_from_semantic_binding_evidence() {
    let descriptor = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let binding = JournalCommit::incremental_through(
        JournalSequence::new(2),
        vec![
            semantic_record(
                2,
                1,
                JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                    session_id: session(),
                }),
            ),
            semantic_record(
                3,
                2,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    1,
                    "codex",
                    "1.0.0",
                    identity("binding"),
                    identity("model"),
                    identity("session"),
                    BindingTransition::new(
                        TransitionMode::Initial,
                        CacheState::NotApplicable,
                        None,
                    ),
                    crate::ContinuationStrategy::BackendManagedState,
                )),
            ),
        ],
    );
    let reader = MemoryReader {
        entries: vec![
            RepositoryEntry::new(RepositorySequence::new(1), record(&descriptor)),
            RepositoryEntry::new(
                RepositorySequence::new(2),
                record_with_discovery(
                    &binding,
                    RecordDiscovery::new(crate::fixture_descriptor(session()))
                        .with_binding_epoch(1),
                ),
            ),
        ],
        missing: false,
    };

    let history = read_stored_session(&reader, session()).unwrap();

    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Consistent
    );
    assert_eq!(
        history.records(),
        &[TranscriptRecord::EventCommitted(
            AgentEvent::SessionCreated {
                session_id: session(),
            }
        )]
    );
    assert_eq!(history.request_trace().len(), 1);
    assert!(matches!(
        history.request_trace()[0].record(),
        StoredRequestTraceRecord::BindingOpened {
            epoch: 1,
            backend_kind,
            ..
        } if backend_kind == "codex"
    ));
}

// semantic 증거가 없는 descriptor-only history에서 summary가
// Anchor를 주장해도 재개 가능으로 추측하지 않고 최초 physical 순번을 보존합니다.
#[test]
fn reports_a_summary_anchor_without_semantic_anchor_evidence() {
    let commit = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let referenced = JournalSequence::new(1);
    let reader = MemoryReader {
        entries: vec![RepositoryEntry::new(
            RepositorySequence::new(1),
            record_with_discovery(
                &commit,
                RecordDiscovery::new(crate::fixture_descriptor(session()))
                    .with_continuation_anchor(referenced),
            ),
        )],
        missing: false,
    };

    let history = read_stored_session(&reader, session()).unwrap();

    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch {
            repository_sequence: RepositorySequence::new(1),
            kind: StoredDiscoveryMismatchKind::ContinuationAnchor { referenced },
        })
    );
    assert!(!history.discovery_consistent());
}

// descriptor-only semantic prefix에는 binding record가 없으므로 epoch만 기록한 summary를
// 근거로 인정하지 않고 해당 physical envelope의 불일치로 보고합니다.
#[test]
fn reports_a_summary_binding_epoch_without_semantic_binding_evidence() {
    let commit = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let reader = MemoryReader {
        entries: vec![RepositoryEntry::new(
            RepositorySequence::new(4),
            record_with_discovery(
                &commit,
                RecordDiscovery::new(crate::fixture_descriptor(session())).with_binding_epoch(7),
            ),
        )],
        missing: false,
    };

    let history = read_stored_session(&reader, session()).unwrap();

    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch {
            repository_sequence: RepositorySequence::new(4),
            kind: StoredDiscoveryMismatchKind::BindingEpoch { claimed: 7 },
        })
    );
}

// tail summary가 비어 있어도 앞 envelope의 근거 없는 Anchor 주장은 사라지지 않으므로
// 전체 history 검증은 처음 잘못된 physical sequence를 계속 보고합니다.
#[test]
fn reports_an_earlier_summary_anchor_even_when_the_tail_clears_it() {
    let descriptor = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let later = JournalCommit::incremental(vec![started(2)]);
    let referenced = JournalSequence::new(1);
    let reader = MemoryReader {
        entries: vec![
            RepositoryEntry::new(
                RepositorySequence::new(1),
                record_with_discovery(
                    &descriptor,
                    RecordDiscovery::new(crate::fixture_descriptor(session()))
                        .with_continuation_anchor(referenced),
                ),
            ),
            RepositoryEntry::new(RepositorySequence::new(2), record(&later)),
        ],
        missing: false,
    };

    let history = read_stored_session(&reader, session()).unwrap();

    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch {
            repository_sequence: RepositorySequence::new(1),
            kind: StoredDiscoveryMismatchKind::ContinuationAnchor { referenced },
        })
    );
}
