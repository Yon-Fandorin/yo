#[cfg(test)]
use std::slice;
use std::{
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{
    ContextActiveSource, JournalDurability, JournalEntry, SemanticRecord, SessionJournal,
    codec::{decode, recover},
    write_state,
};
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, BackendBindingEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, JournalSequence, SessionId, TurnId, TurnOutcome, TurnRef, UserInput,
    session_repository::{
        AppendError, AppendReceipt, DurableCutoff, DurableRecord, DurableRecordKind,
        RepositoryEntry, RepositoryError, RepositorySequence, SessionRepository, StoragePressure,
        StoragePressureCause,
    },
};

mod durable_messages;
mod gap_recovery;

// 복구된 의미 기록은 codec 전용 기록을 제외해 sequence가 성길 수 있으므로,
// cursor를 배열 index로 쓰지 않고 실제 좌표 뒤의 첫 이벤트부터 전달한다.
#[test]
fn transcript_cursor_preserves_events_after_sparse_recovery() {
    let mut journal = SessionJournal::new();
    let session_id = session(1);
    let prior = AgentEvent::SessionCreated { session_id };
    write_state(&journal.state).entries = vec![
        JournalEntry::new(
            JournalSequence::new(5),
            SemanticRecord::EventCommitted(prior),
        ),
        JournalEntry::new(
            JournalSequence::new(9),
            SemanticRecord::EventCommitted(AgentEvent::TurnFinished {
                turn: turn(session_id, 1),
                outcome: TurnOutcome::Completed,
            }),
        ),
    ];
    let reader = journal.transcript_reader();
    let cursor = reader.head_sequence();
    let started = AgentEvent::TurnStarted {
        turn: turn(session_id, 2),
    };
    journal.append_events(slice::from_ref(&started));

    let resumed = reader.read_after(cursor);
    assert_eq!(resumed.entries().len(), 1);
    assert_eq!(resumed.entries()[0].sequence(), JournalSequence::new(10));
    assert_eq!(
        resumed.entries()[0].record(),
        &crate::TranscriptRecord::EventCommitted(started)
    );
    assert!(reader.read_after(resumed.head()).entries().is_empty());
    assert_eq!(
        reader
            .read_after(Some(JournalSequence::new(6)))
            .entries()
            .len(),
        2
    );
    assert!(
        reader
            .read_after(Some(JournalSequence::new(u64::MAX)))
            .entries()
            .is_empty()
    );
}

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
    let recovered = recover(slice::from_ref(&commit)).unwrap();
    assert_eq!(recovered.descriptor(), Some(&descriptor));
    assert_eq!(
        recovered.journal_cutoff().map(JournalSequence::get),
        Some(4)
    );
}

// 준비된 상관 제출은 내구성 append에 성공한 경우에만 정확한 SubmissionId와 함께 live Journal에
// 반영됩니다.
#[test]
fn publishes_correlated_submission_transactionally_with_its_exact_identity() {
    let session_id = session(30);
    let descriptor = crate::fixture_descriptor(session_id);
    let repository = SharedRepository::default();
    let pressure = Arc::clone(&repository.pressure);
    let observed = Arc::clone(&repository.state);
    let mut journal =
        SessionJournal::with_repository_and_descriptor(Box::new(repository), descriptor);
    journal.initialize_durability();
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );

    let turn = turn(session_id, 1);
    let command = AgentCommand::SteerTurn {
        turn,
        input: UserInput::from("correct this"),
    };
    let id: crate::SubmissionId = "10000000-0000-4000-8000-000000000030".parse().unwrap();
    let before = journal.entries();
    pressure.store(true, Ordering::Release);

    assert!(!journal.append_committed_submission_transactionally(command.clone(), id, &[]));
    assert_eq!(journal.entries(), before);

    pressure.store(false, Ordering::Release);
    assert!(journal.append_committed_submission_transactionally(command.clone(), id, &[]));
    let entries = journal.entries();
    let Some(SemanticRecord::CommandCommitted(committed)) =
        entries.last().map(JournalEntry::record)
    else {
        panic!("the durable submission is the latest semantic record");
    };
    assert_eq!(committed.command(), &command);
    assert_eq!(committed.submission_id(), Some(id));

    let stored = observed.lock().unwrap();
    let commit = decode(
        stored
            .entries
            .last()
            .expect("the successful transaction reaches durable storage")
            .record()
            .payload(),
    )
    .expect("the transaction's durable record decodes");
    assert!(commit.records().iter().any(|record| {
        matches!(
            record.record(),
            super::codec::JournalRecord::CommandCommitted(committed)
                if committed.command() == &command && committed.submission_id() == Some(id)
        )
    }));
}

// 전달 시점에 갱신된 active suffix는 source boundary 안의 모든 제출을 정확한 순서와 횟수로 담아야
// 합니다.
#[test]
fn advances_live_active_suffix_only_for_exact_committed_input_provenance() {
    let session_id = session(31);
    let turn = turn(session_id, 1);
    let initial = UserInput::new("initial");
    let first_correction = UserInput::new("first correction");
    let second_correction = UserInput::new("second correction");
    let submission_id = |value: u8| -> crate::SubmissionId {
        format!("10000000-0000-4000-8000-0000000000{value:02x}")
            .parse()
            .unwrap()
    };
    let mut journal = SessionJournal::new();
    journal.append_committed_submission(
        AgentCommand::StartTurn {
            turn,
            input: initial.clone(),
        },
        submission_id(31),
        &[],
    );
    let first_sequence = journal.last_sequence().unwrap();
    journal.append_committed_submission(
        AgentCommand::SteerTurn {
            turn,
            input: first_correction.clone(),
        },
        submission_id(32),
        &[],
    );
    journal.append_committed_submission(
        AgentCommand::SteerTurn {
            turn,
            input: second_correction.clone(),
        },
        submission_id(33),
        &[],
    );
    let last_sequence = journal.last_sequence().unwrap();
    let mut source = ContextActiveSource::new(
        turn,
        first_sequence,
        first_sequence,
        vec![initial.model_replay_item()],
    );
    let exact_items = vec![
        initial.model_replay_item(),
        first_correction.model_replay_item(),
        second_correction.model_replay_item(),
    ];

    assert!(!journal.advance_active_context_source(
        &mut source,
        turn,
        last_sequence,
        vec![
            exact_items[0].clone(),
            exact_items[2].clone(),
            exact_items[1].clone(),
        ],
    ));
    assert!(journal.advance_active_context_source(&mut source, turn, last_sequence, exact_items,));
}

// accepted request 뒤 아직 model response Activity가 오지 않았다면 correction만으로든
// 꾸며낸 assistant item과 함께든 live active source를 그 너머로 옮길 수 없습니다.
#[test]
fn does_not_advance_live_active_source_past_an_open_accepted_request() {
    let session_id = session(31);
    let turn = turn(session_id, 1);
    let initial = UserInput::new("initial");
    let correction = UserInput::new("correction");
    let submission_id = |value: u8| -> crate::SubmissionId {
        format!("10000000-0000-4000-8000-0000000000{value:02x}")
            .parse()
            .unwrap()
    };
    let mut journal = SessionJournal::new();
    journal.append_committed_submission(
        AgentCommand::StartTurn {
            turn,
            input: initial.clone(),
        },
        submission_id(31),
        &[],
    );
    let first_sequence = journal.last_sequence().unwrap();
    let mut source = ContextActiveSource::new(
        turn,
        first_sequence,
        first_sequence,
        vec![initial.model_replay_item()],
    );
    journal.append_accepted_request(
        turn,
        1,
        1,
        BackendRequestEvidence::new(
            "scripted/request/v1",
            BackendIdentity::new("scripted/exchange/v1", "exchange-1"),
            BackendIdentity::new("scripted/request/v1", "request-1"),
        ),
    );
    journal.append_committed_submission(
        AgentCommand::SteerTurn {
            turn,
            input: correction.clone(),
        },
        submission_id(32),
        &[],
    );
    let last_sequence = journal.last_sequence().unwrap();

    assert!(!journal.advance_active_context_source(
        &mut source,
        turn,
        last_sequence,
        vec![initial.model_replay_item(), correction.model_replay_item()],
    ));
    assert!(!journal.advance_active_context_source(
        &mut source,
        turn,
        last_sequence,
        vec![
            initial.model_replay_item(),
            crate::ModelReplayItem::Message {
                role: crate::ModelReplayRole::Assistant,
                content: "unobserved response".to_owned(),
                refusal: None,
            },
            correction.model_replay_item(),
        ],
    ));
}

// assistant ActivityFinished 뒤 suffix 전달 전에 도착한 correction도 FIFO 그대로 같은
// active source boundary에 반영할 수 있습니다.
#[test]
fn advances_live_active_source_after_a_completed_response_with_a_correction() {
    let session_id = session(31);
    let turn = turn(session_id, 1);
    let initial = UserInput::new("initial");
    let correction = UserInput::new("correction");
    let submission_id = |value: u8| -> crate::SubmissionId {
        format!("10000000-0000-4000-8000-0000000000{value:02x}")
            .parse()
            .unwrap()
    };
    let mut journal = SessionJournal::new();
    journal.append_committed_submission(
        AgentCommand::StartTurn {
            turn,
            input: initial.clone(),
        },
        submission_id(31),
        &[],
    );
    let first_sequence = journal.last_sequence().unwrap();
    let mut source = ContextActiveSource::new(
        turn,
        first_sequence,
        first_sequence,
        vec![initial.model_replay_item()],
    );
    journal.append_accepted_request(
        turn,
        1,
        1,
        BackendRequestEvidence::new(
            "scripted/request/v1",
            BackendIdentity::new("scripted/exchange/v1", "exchange-1"),
            BackendIdentity::new("scripted/request/v1", "request-1"),
        ),
    );
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    journal.append_events(&[
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        },
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        },
    ]);
    journal.append_committed_submission(
        AgentCommand::SteerTurn {
            turn,
            input: correction.clone(),
        },
        submission_id(32),
        &[],
    );
    let last_sequence = journal.last_sequence().unwrap();

    assert!(journal.advance_active_context_source(
        &mut source,
        turn,
        last_sequence,
        vec![
            initial.model_replay_item(),
            crate::ModelReplayItem::Message {
                role: crate::ModelReplayRole::Assistant,
                content: "completed response".to_owned(),
                refusal: None,
            },
            correction.model_replay_item(),
        ],
    ));
}

// live writer가 binding, accepted request, 완료 outcome과 Anchor를 각각 한 physical append로
// 내보내면 repository discovery와 전체 recovery가 같은 최신 Anchor를 가리키는지 확인한다.
#[test]
fn durable_live_correlation_publishes_one_recoverable_anchor() {
    let session_id = session(1);
    let descriptor = crate::fixture_descriptor(session_id);
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal =
        SessionJournal::with_repository_and_descriptor(Box::new(repository), descriptor);
    journal.initialize_durability();
    journal.append_initial_binding(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
        1,
        BackendBindingEvidence::new(
            "scripted",
            "1",
            BackendIdentity::new("scripted/binding/v1", "binding-1"),
            BackendIdentity::new("scripted/model/v1", "model-1"),
            BackendIdentity::new("scripted/session/v1", "session-1"),
            crate::ContinuationStrategy::BackendManagedState,
        ),
    );
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::MIN));
    let submission_id = "10000000-0000-4000-8000-000000000019"
        .parse()
        .expect("the test submission fixture is a UUIDv4");
    let accepted = journal.append_accepted_submission(
        AgentCommand::StartTurn {
            turn,
            input: UserInput::from("persist anchor"),
        },
        submission_id,
        &[AgentEvent::TurnStarted { turn }],
        1,
        None,
        BackendRequestEvidence::new(
            "scripted/request/v1",
            BackendIdentity::new("scripted/exchange/v1", "exchange-1"),
            BackendIdentity::new("scripted/request/v1", "request-1"),
        ),
    );
    journal.append_resumable_turn(
        &AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Completed,
        },
        1,
        None,
        accepted,
        crate::ContinuationStrategy::BackendManagedState,
        BackendOutcomeEvidence::without_identity(),
    );

    let state = observed.lock().unwrap();
    assert_eq!(state.entries.len(), 4);
    let durable_anchor = state.entries[3]
        .record()
        .discovery()
        .and_then(|discovery| discovery.continuation_anchor())
        .expect("the completed physical append must publish its Anchor");
    let commits = state
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.continuation_anchor(), Some(durable_anchor));
    assert_eq!(recovered.binding_epoch(), Some(1));
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
