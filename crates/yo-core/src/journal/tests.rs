use std::{
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{
    JournalDurability, SemanticRecord, SessionJournal,
    codec::{decode, recover},
};
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, JournalSequence, SessionId, TurnId, TurnRef, UserInput,
    session_repository::{
        AppendError, AppendReceipt, DurableCutoff, DurableRecord, DurableRecordKind,
        RepositoryEntry, RepositoryError, RepositorySequence, SessionRepository, StoragePressure,
        StoragePressureCause,
    },
};

mod durable_messages;
mod gap_recovery;

#[derive(Default)]
struct RepositoryState {
    entries: Vec<RepositoryEntry>,
}

#[derive(Clone)]
struct SnapshotGateRepository {
    state: Arc<Mutex<RepositoryState>>,
    fail_on_append: usize,
    attempts: Arc<Mutex<usize>>,
}

impl SessionRepository for SnapshotGateRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts == self.fail_on_append {
            let state = self.state.lock().unwrap();
            return Err(AppendError::SnapshotRequired {
                durable_cutoff: state
                    .entries
                    .last()
                    .map_or(DurableCutoff::KnownEmpty, |entry| DurableCutoff::Known {
                        journal_sequence: entry.record().journal_cutoff(),
                        repository_sequence: entry.sequence(),
                    }),
            });
        }
        let mut state = self.state.lock().unwrap();
        let sequence = RepositorySequence::new(
            u64::try_from(state.entries.len()).expect("test entries fit u64") + 1,
        );
        state.entries.push(RepositoryEntry::new(sequence, record));
        Ok(AppendReceipt::new(sequence))
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

#[derive(Clone, Default)]
struct SharedRepository {
    state: Arc<Mutex<RepositoryState>>,
    pressure: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct TransientReadRepository {
    state: Arc<Mutex<RepositoryState>>,
    fail_next_read: Arc<AtomicBool>,
}

impl SessionRepository for TransientReadRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let mut state = self.state.lock().unwrap();
        let sequence = RepositorySequence::new(
            u64::try_from(state.entries.len()).expect("test entries fit u64") + 1,
        );
        state.entries.push(RepositoryEntry::new(sequence, record));
        Ok(AppendReceipt::new(sequence))
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        if self.fail_next_read.swap(false, Ordering::AcqRel) {
            return Err(RepositoryError::Unavailable {
                message: "temporary test read failure".to_owned(),
            });
        }
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

impl SessionRepository for SharedRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let mut state = self.state.lock().unwrap();
        if self.pressure.load(Ordering::Acquire) {
            let durable_cutoff = state
                .entries
                .last()
                .map_or(DurableCutoff::KnownEmpty, |entry| DurableCutoff::Known {
                    journal_sequence: entry.record().journal_cutoff(),
                    repository_sequence: entry.sequence(),
                });
            return Err(AppendError::StoragePressure {
                pressure: StoragePressure::new(durable_cutoff, StoragePressureCause::Capacity),
                source: None,
            });
        }
        let sequence = RepositorySequence::new(
            u64::try_from(state.entries.len()).expect("test entries fit u64") + 1,
        );
        state.entries.push(RepositoryEntry::new(sequence, record));
        Ok(AppendReceipt::new(sequence))
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

// persistent Session 초기화는 backend CreateSession보다 먼저 descriptor-only envelope를
// 기록하고, 이 physical record에는 아직 semantic JournalSequence가 없어야 한다.
#[test]
fn initializes_persistence_with_a_descriptor_only_envelope() {
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let descriptor = crate::fixture_descriptor(crate::fixture_session(1));
    let mut journal =
        SessionJournal::with_repository_and_descriptor(Box::new(repository), descriptor.clone());

    journal.initialize_durability();

    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence,
        } if repository_sequence.get() == 1
    ));
    let state = observed.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].record().journal_cutoff(), None);
    let commit = decode(state.entries[0].record().payload()).unwrap();
    assert!(matches!(
        commit.records()[0].record(),
        super::codec::JournalRecord::SessionDescriptor(observed) if observed == &descriptor
    ));
}

// 첫 descriptor append가 용량 압력으로 실패해도 Session은 memory-only로 계속되고,
// 공간이 돌아오면 descriptor와 그동안의 semantic prefix를 한 snapshot으로 먼저 복구한다.
#[test]
fn recovers_an_initial_descriptor_gap_with_the_complete_session_snapshot() {
    let session_id = crate::fixture_session(1);
    let repository = SharedRepository::default();
    repository.pressure.store(true, Ordering::Release);
    let pressure = Arc::clone(&repository.pressure);
    let observed = Arc::clone(&repository.state);
    let descriptor = crate::fixture_descriptor(session_id);
    let mut journal =
        SessionJournal::with_repository_and_descriptor(Box::new(repository), descriptor.clone());

    journal.initialize_durability();
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Gap {
            durable_cutoff: DurableCutoff::KnownEmpty,
            cause: super::DurabilityGapCause::Capacity,
        }
    ));

    pressure.store(false, Ordering::Release);
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::MIN));
    journal.append_committed_submission(
        AgentCommand::StartTurn {
            turn,
            input: UserInput::from("recover"),
        },
        "10000000-0000-4000-8000-000000000012"
            .parse()
            .expect("the test submission fixture is a UUIDv4"),
        &[AgentEvent::TurnStarted { turn }],
    );

    let state = observed.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(
        state.entries[0].record().kind(),
        DurableRecordKind::Snapshot
    );
    let commit = decode(state.entries[0].record().payload()).unwrap();
    let recovered = recover(std::slice::from_ref(&commit)).unwrap();
    assert_eq!(recovered.descriptor(), Some(&descriptor));
    assert_eq!(
        recovered.journal_cutoff().map(JournalSequence::get),
        Some(4)
    );
}

fn session(value: u64) -> SessionId {
    crate::fixture_session(value)
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(session_id, TurnId::new(NonZeroU64::new(value).unwrap()))
}

// 한 명령이 만든 의미 이벤트를 함께 기록하면 명령이 먼저 나오고 이어지는 이벤트들이
// 같은 batch 순서를 유지해야 replay가 원래의 원인과 결과를 복원할 수 있다.
#[test]
fn appends_a_committed_command_before_its_committed_events() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let command = AgentCommand::StartTurn {
        turn,
        input: UserInput::new("inspect the repository"),
    };
    let events = vec![AgentEvent::TurnStarted { turn }];
    let mut journal = SessionJournal::new();

    let submission_id = crate::SubmissionId::new().unwrap();
    journal.append_committed_submission(command.clone(), submission_id, &events);

    assert_eq!(journal.entries().len(), 2);
    assert_eq!(
        journal.entries()[0].record(),
        &SemanticRecord::CommandCommitted(
            super::CommittedCommand::submission(command, submission_id).unwrap()
        )
    );
    assert_eq!(
        journal.entries()[1].record(),
        &SemanticRecord::EventCommitted(events[0].clone())
    );
}

// 서로 다른 append에서 추가된 기록도 1부터 시작하는 하나의 연속 sequence를 공유해야
// frontend와 저장소가 누락이나 중복 없이 suffix를 요청할 수 있다.
#[test]
fn assigns_one_contiguous_sequence_across_append_boundaries() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let mut journal = SessionJournal::new();

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::TurnStarted { turn }]);

    let sequences = journal
        .entries()
        .iter()
        .map(|entry| entry.sequence().get())
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2, 3]);
}

// 빈 event batch는 sequence를 소비하거나 가짜 기록을 만들지 않아야 다음 실제 의미
// 이벤트의 번호가 이전 기록 바로 다음으로 이어진다.
#[test]
fn empty_event_batches_do_not_create_observations_or_sequence_gaps() {
    let session_id = session(1);
    let mut journal = SessionJournal::new();

    journal.append_events(&[]);
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );

    let sequences = journal
        .entries()
        .iter()
        .map(|entry| entry.sequence().get())
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2]);
}
