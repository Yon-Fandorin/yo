use std::num::NonZeroU64;

use super::*;
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent,
    JournalSequence, TranscriptRecord, TurnId, TurnRef,
    journal::codec::{
        BackendBindingOpened, BindingTransition, CacheState, JournalCommit, JournalCommitKind,
        JournalRecord, MessageEnded, MessageOutcome, MessageReset, MessageSegment, MessageStream,
        MessageTerminal, ReplaySequence, SequencedJournalRecord, TransitionMode, VersionedIdentity,
        encode,
    },
    session_repository::{
        DurableRecord, RecordDiscovery, RepositoryEntry, RepositoryError, RepositorySequence,
        StoredSession,
    },
};

#[derive(Debug, Default)]
struct MemoryReader {
    entries: Vec<RepositoryEntry>,
    missing: bool,
}

impl StoredSessionReader for MemoryReader {
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        Ok(Vec::new())
    }

    fn read_session(
        &self,
        _session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        Ok(if self.missing {
            StoredSessionSnapshot::Missing
        } else {
            StoredSessionSnapshot::Present(self.entries.clone())
        })
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

fn session() -> SessionId {
    crate::fixture_session(1)
}

fn activity() -> ActivityRef {
    ActivityRef::new(
        TurnRef::new(session(), TurnId::new(NonZeroU64::new(2).unwrap())),
        ActivityId::new(NonZeroU64::new(3).unwrap()),
    )
}

fn record(commit: &JournalCommit) -> DurableRecord {
    record_with_discovery(
        commit,
        RecordDiscovery::new(crate::fixture_descriptor(session())),
    )
}

fn record_with_discovery(commit: &JournalCommit, discovery: RecordDiscovery) -> DurableRecord {
    let record = match commit.kind() {
        JournalCommitKind::Incremental => DurableRecord::incremental(encode(commit).unwrap()),
        JournalCommitKind::Snapshot => DurableRecord::snapshot(encode(commit).unwrap()),
    };
    record
        .with_journal_cutoff(commit.journal_cutoff())
        .with_discovery(discovery)
}

fn reader(commits: &[JournalCommit]) -> MemoryReader {
    MemoryReader {
        entries: commits
            .iter()
            .enumerate()
            .map(|(index, commit)| {
                RepositoryEntry::new(
                    RepositorySequence::new(u64::try_from(index).unwrap() + 1),
                    record(commit),
                )
            })
            .collect(),
        missing: false,
    }
}

fn started(sequence: u64) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity: activity(),
            kind: ActivityKind::AgentMessage,
        }),
    )
}

fn finished(sequence: u64, outcome: ActivityOutcome) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity: activity(),
            outcome,
        }),
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

// 여러 physical segment와 그 앞의 오래된 revision은 저장 최적화 경계일 뿐이므로,
// replacement revision의 최종 text snapshot 하나와 종료 event만 frontend에 전달한다.
#[test]
fn coalesces_segments_and_superseded_revisions() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "old".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(4),
            JournalRecord::MessageReset(MessageReset::new(activity(), MessageStream::Agent, 2)),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(5),
            JournalRecord::MessageSegment(MessageSegment::for_revision(
                activity(),
                MessageStream::Agent,
                2,
                1,
                "new ".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(6),
            JournalRecord::MessageEnded(MessageTerminal::new(
                Some(MessageSegment::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    2,
                    "answer".to_owned(),
                )),
                MessageEnded::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    MessageOutcome::Completed,
                    2,
                    10,
                ),
            )),
        ),
        finished(7, ActivityOutcome::Completed),
    ]);

    let history = read_stored_session(&reader(&[commit]), session()).unwrap();
    let updates = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update, .. }) => {
                Some(update)
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        updates,
        [&ActivityUpdate::TextSnapshot("new answer".to_owned())]
    );
    assert!(history.discovery_consistent());
    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Consistent
    );
    assert_eq!(history.recovery(), StoredSessionRecovery::NotRequired);
    assert_eq!(history.continuity(), StoredSessionContinuity::NotObservable);
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

// 이전 revision에 text가 있더라도 다음 revision의 zero-byte terminal은 "빈 답변"이라는
// authoritative snapshot이므로, 저장된 Chat에 오래된 text를 남기지 않습니다.
#[test]
fn empty_terminal_revision_clears_superseded_text() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "superseded".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(4),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(5, ActivityOutcome::Completed),
    ]);

    let history = read_stored_session(&reader(&[commit]), session()).unwrap();
    let updates = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update, .. }) => {
                Some(update)
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(updates, [&ActivityUpdate::TextSnapshot(String::new())]);
}

// crash가 열린 message의 terminal을 쓰기 전에 멈추면 recovery seal을 버리지 않고
// 마지막 text 뒤에 interrupted ActivityFinished를 붙여 완료된 대화처럼 보이지 않게 한다.
#[test]
fn projects_an_open_message_as_explicitly_interrupted() {
    let descriptor = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let open = JournalCommit::incremental(vec![
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "partial".to_owned(),
            )),
        ),
    ]);

    let history = read_stored_session(&reader(&[descriptor, open]), session()).unwrap();

    assert_eq!(history.recovery(), StoredSessionRecovery::Interrupted);
    assert!(matches!(
        history.records().last(),
        Some(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                outcome: ActivityOutcome::Interrupted,
                ..
            }
        ))
    ));
}

// message terminal과 semantic ActivityFinished의 outcome이 다르면 손상된 history를 일부
// 출력하지 않고 두 authority가 충돌한다는 복구 오류로 거부한다.
#[test]
fn rejects_conflicting_message_and_activity_outcomes() {
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
        finished(4, ActivityOutcome::Interrupted),
    ]);

    let error = read_stored_session(&reader(&[commit]), session()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("conflicting message and activity outcomes")
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
