use std::{
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::{
    ActivityId, ActivityRef, AgentCommand, AgentEvent, JournalSequence, TurnId, TurnRef, UserInput,
    journal::codec::{
        BackendBindingOpened, BindingTransition, CacheState, JournalRecord, MessageEnded,
        MessageOutcome, MessageSegment, MessageSegmenter, MessageStream, MessageTerminal,
        ReplaySequence, SequencedJournalRecord, TransitionMode, VersionedIdentity,
    },
    session_repository::{LocalSessionRepository, RepositoryEntry, RepositorySequence},
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-journal-repository-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("the test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Default)]
struct RecordingRepository {
    appended: Vec<(SessionId, DurableRecord)>,
    entries: Vec<RepositoryEntry>,
    fail_append: bool,
}

impl SessionRepository for RecordingRepository {
    fn append(
        &mut self,
        session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        if self.fail_append {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "injected append failure".to_owned(),
            }));
        }
        let sequence = RepositorySequence::new(
            u64::try_from(self.appended.len()).expect("test append count fits u64") + 1,
        );
        self.entries
            .push(RepositoryEntry::new(sequence, record.clone()));
        self.appended.push((session_id, record));
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

fn repository_with_descriptor<R>(repository: R) -> JournalRepository<R>
where
    R: SessionRepository,
{
    let mut repository = JournalRepository::new(repository);
    repository
        .append(
            session(),
            &JournalCommit::descriptor(crate::fixture_descriptor(session())),
        )
        .expect("the Session descriptor persists first");
    repository
}

fn activity() -> ActivityRef {
    let turn = TurnRef::new(session(), TurnId::new(NonZeroU64::new(2).unwrap()));
    ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(3).unwrap()))
}

fn command_commit() -> JournalCommit {
    command_commit_at(2)
}

fn command_commit_at(first_sequence: u64) -> JournalCommit {
    let session_id = session();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    JournalCommit::incremental(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(first_sequence),
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: UserInput::new("inspect"),
                    },
                    crate::SubmissionId::from_uuid(
                        uuid::Builder::from_random_bytes(
                            [u8::try_from(first_sequence)
                                .expect("the test replay sequence fits in one byte");
                                16],
                        )
                        .into_uuid(),
                    )
                    .expect("the test submission fixture is a UUIDv4"),
                )
                .unwrap(),
            ),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(first_sequence + 1),
            JournalRecord::EventCommitted(AgentEvent::TurnStarted { turn }),
        ),
    ])
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

fn test_identity(name: &str) -> VersionedIdentity {
    VersionedIdentity::new(format!("yo.test.{name}/v1"), format!("{name}:value"))
}

fn unterminated_snapshot() -> JournalCommit {
    JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(2),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "partial".to_owned(),
            )),
        ),
    ])
}

fn snapshot_without_durable_message() -> JournalCommit {
    JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(2),
            JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                session_id: session(),
            }),
        ),
    ])
}

fn message_segment_commit(sequence: u64) -> JournalCommit {
    JournalCommit::incremental(vec![SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::MessageSegment(MessageSegment::new(
            activity(),
            MessageStream::Agent,
            1,
            "partial".to_owned(),
        )),
    )])
}

fn terminal_commit(sequence: u64, segment_count: u64) -> JournalCommit {
    JournalCommit::incremental(vec![SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::MessageEnded(MessageTerminal::new(
            None,
            MessageEnded::new(
                activity(),
                MessageStream::Agent,
                MessageOutcome::Completed,
                segment_count,
                7,
            ),
        )),
    )])
}

// command와 결과 event로 된 semantic commit 하나를 저장하면 repository append도 정확히
// 한 번만 호출되고 envelope의 Journal cutoff는 physical sequence와 별도로 2를 보존해야 한다.
#[test]
fn maps_one_semantic_commit_to_one_physical_append() {
    let mut repository = repository_with_descriptor(RecordingRepository::default());

    let receipt = repository
        .append(session(), &command_commit())
        .expect("the semantic commit persists");
    let inner = repository.into_inner();

    assert_eq!(receipt.sequence().get(), 2);
    assert_eq!(inner.appended.len(), 2);
    assert_eq!(
        inner.appended[1]
            .1
            .journal_cutoff()
            .map(JournalSequence::get),
        Some(3)
    );
    assert_eq!(
        decode(inner.appended[1].1.payload()).expect("payload is a Journal commit"),
        command_commit()
    );
}

// binding-open semantic record가 저장된 append의 discovery에는 recovery가 검증한 현재
// epoch가 투영되어야 목록 reader가 payload를 추측하지 않고 빠른 후보 탐색을 할 수 있습니다.
#[test]
fn projects_the_recovered_binding_epoch_into_discovery() {
    let mut repository = repository_with_descriptor(RecordingRepository::default());
    let commit = JournalCommit::incremental_through(
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
                    test_identity("binding"),
                    test_identity("model"),
                    test_identity("session"),
                    BindingTransition::new(
                        TransitionMode::Initial,
                        CacheState::NotApplicable,
                        None,
                    ),
                )),
            ),
        ],
    );

    repository
        .append(session(), &commit)
        .expect("the validated binding persists");
    let inner = repository.into_inner();

    assert_eq!(
        inner.appended[1]
            .1
            .discovery()
            .expect("durable records include discovery")
            .binding_epoch(),
        Some(1)
    );
}

// 빈 repository의 첫 incremental이 Journal sequence 1이 아니면 성공 뒤 복구 불가능한
// 로그가 되므로 semantic prefix 검증이 physical append 전에 거부해야 한다.
#[test]
fn rejects_a_noninitial_sequence_before_the_first_physical_append() {
    let mut repository = JournalRepository::new(RecordingRepository::default());

    let error = repository
        .append(session(), &command_commit_at(4))
        .expect_err("the first durable Journal record must begin at sequence 1");
    let inner = repository.into_inner();

    assert!(matches!(error, JournalRepositoryError::Codec(_)));
    assert!(inner.appended.is_empty());
}

// sequence 1~2를 저장한 뒤 4부터 시작하는 commit을 붙이면 durable gap이 생기므로 두
// commit을 함께 검증해 두 번째 append를 거부하고 기존 prefix는 계속 복구해야 한다.
#[test]
fn rejects_a_sequence_gap_against_the_durable_prefix() {
    let mut repository = repository_with_descriptor(RecordingRepository::default());
    repository
        .append(session(), &command_commit_at(2))
        .expect("the contiguous prefix persists");

    let error = repository
        .append(session(), &command_commit_at(5))
        .expect_err("the candidate cannot skip replay sequence 4");
    let recovered = repository
        .recover(session())
        .expect("the rejected candidate leaves the prefix recoverable");
    let inner = repository.into_inner();

    assert!(matches!(error, JournalRepositoryError::Codec(_)));
    assert_eq!(recovered.records().len(), 3);
    assert_eq!(inner.appended.len(), 2);
}

// durable segment 하나 뒤에 전체 count를 2라고 주장하는 terminal을 붙이면 앞 commit과
// 맞지 않으므로 candidate를 append하지 않고 열린 prefix의 interrupted recovery를 보존한다.
#[test]
fn rejects_a_terminal_that_mismatches_the_durable_prefix() {
    let mut repository = repository_with_descriptor(RecordingRepository::default());
    repository
        .append(session(), &message_segment_commit(2))
        .expect("the bounded segment persists");

    let error = repository
        .append(session(), &terminal_commit(3, 2))
        .expect_err("the terminal counts must match the durable prefix");
    repository
        .append(session(), &terminal_commit(3, 1))
        .expect("a valid replacement candidate still uses the unchanged recovered prefix");
    let recovered = repository
        .recover(session())
        .expect("the rejected terminal leaves an interruptible prefix");
    let inner = repository.into_inner();

    assert!(matches!(error, JournalRepositoryError::Codec(_)));
    assert!(recovered.recovery_commit().is_none());
    assert_eq!(inner.appended.len(), 3);
}

// 열린 message의 pending text가 경계에서 segment로 먼저 저장된 뒤에는 command가 같은
// Session 순서로 이어질 수 있어야 한다. crash recovery는 command를 지우지 않고 열린
// message에 interrupted seal을 제안한다.
#[test]
fn accepts_a_command_after_a_forced_message_segment() {
    let mut repository = repository_with_descriptor(RecordingRepository::default());
    repository
        .append(session(), &message_segment_commit(2))
        .expect("the bounded segment persists");

    repository
        .append(session(), &command_commit_at(3))
        .expect("the forced segment establishes the non-text ordering boundary");
    let recovered = repository
        .recover(session())
        .expect("the accepted command leaves an interruptible message prefix");
    let inner = repository.into_inner();

    assert!(recovered.recovery_commit().is_some());
    assert_eq!(inner.appended.len(), 3);
}

// 종료 직전의 non-empty tail을 MessageEnded record 안에 넣어 저장하면 둘이 하나의
// repository envelope에 함께 들어가 어떤 caller도 절반만 durable하게 만들 수 없어야 한다.
#[test]
fn persists_the_final_message_tail_and_terminal_seal_atomically() {
    let session_id = session();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    let activity =
        crate::ActivityRef::new(turn, crate::ActivityId::new(NonZeroU64::new(3).unwrap()));
    let mut segmenter = MessageSegmenter::new(activity, MessageStream::Agent);
    segmenter.push_text("final tail", Duration::ZERO);
    let commit = JournalCommit::incremental(vec![SequencedJournalRecord::new(
        JournalSequence::new(2),
        segmenter.finish(MessageOutcome::Completed),
    )]);
    let mut repository = repository_with_descriptor(RecordingRepository::default());

    repository
        .append(session_id, &commit)
        .expect("the terminal commit persists");
    let inner = repository.into_inner();
    let decoded =
        decode(inner.appended[1].1.payload()).expect("the physical payload is semantic JSON");

    assert_eq!(inner.appended.len(), 2);
    assert_eq!(decoded.records().len(), 1);
    let JournalRecord::MessageEnded(terminal) = decoded.records()[0].record() else {
        panic!("the terminal record remains atomic");
    };
    assert_eq!(
        terminal
            .final_segment()
            .expect("the final tail is embedded")
            .text(),
        "final tail"
    );
}

// physical append가 실패하면 adapter가 성공 receipt를 만들거나 두 번째 append로 일부를
// 재시도하지 않아야 호출자가 semantic 결과를 volatile로 명시적으로 처리할 수 있다.
#[test]
fn leaves_a_failed_semantic_commit_unpublished_by_the_durable_adapter() {
    let mut repository = repository_with_descriptor(RecordingRepository::default());
    repository.repository.fail_append = true;

    let error = repository
        .append(session(), &command_commit())
        .expect_err("the injected append failure remains visible");
    let inner = repository.into_inner();

    assert!(matches!(error, JournalRepositoryError::Append(_)));
    assert_eq!(inner.appended.len(), 1);
}

// complete snapshot이 sequence 1부터 시작하지 않으면 Local repository의 snapshot gate를
// 해제하기 전에 adapter가 거부하고 physical append를 전혀 호출하지 않아야 한다.
#[test]
fn rejects_an_incomplete_snapshot_before_physical_append() {
    let commit = JournalCommit::snapshot(vec![SequencedJournalRecord::new(
        JournalSequence::new(5),
        JournalRecord::EventCommitted(AgentEvent::SessionCreated {
            session_id: session(),
        }),
    )]);
    let mut repository = JournalRepository::new(RecordingRepository::default());

    let error = repository
        .append(session(), &commit)
        .expect_err("an incomplete snapshot cannot clear the recovery gate");
    let inner = repository.into_inner();

    assert!(matches!(error, JournalRepositoryError::Codec(_)));
    assert!(inner.appended.is_empty());
}

// reopen gate에서 sequence 1이지만 terminal seal 없는 snapshot을 시도하면 adapter가
// physical append 전에 거부하고, 뒤의 incremental도 계속 SnapshotRequired여야 한다.
#[test]
fn unterminated_snapshot_cannot_release_the_local_recovery_gate() {
    let directory = TestDirectory::new("unterminated-snapshot-gate");
    {
        let local = LocalSessionRepository::open(directory.path(), 32_768)
            .expect("the local repository opens");
        let mut repository = repository_with_descriptor(local);
        repository
            .append(session(), &command_commit_at(2))
            .expect("the initial semantic commit persists");
    }
    let local = LocalSessionRepository::open(directory.path(), 32_768)
        .expect("the local repository reopens with a snapshot gate");
    let mut repository = JournalRepository::new(local);

    let snapshot_error = repository
        .append(session(), &unterminated_snapshot())
        .expect_err("the incomplete snapshot is rejected before local append");
    let later_error = repository
        .append(session(), &command_commit_at(4))
        .expect_err("the rejected snapshot cannot release the local gate");

    assert!(matches!(snapshot_error, JournalRepositoryError::Codec(_)));
    assert!(
        matches!(
            later_error,
            JournalRepositoryError::Append(AppendError::SnapshotRequired { .. })
        ),
        "unexpected later error: {later_error:?}"
    );
}

// reopen 전 durable segment를 생략한 채 다른 sequence-1 event만 담은 snapshot은 자체로는
// 완결돼 보여도 partial message와 interrupted seal을 잃으므로 거부하고 gate를 유지해야 한다.
#[test]
fn replacement_snapshot_cannot_erase_an_open_durable_message() {
    let directory = TestDirectory::new("snapshot-preserves-open-message");
    {
        let local = LocalSessionRepository::open(directory.path(), 32_768)
            .expect("the local repository opens");
        let mut repository = repository_with_descriptor(local);
        repository
            .append(session(), &message_segment_commit(2))
            .expect("the durable message prefix persists");
    }
    let local = LocalSessionRepository::open(directory.path(), 32_768)
        .expect("the local repository reopens with a snapshot gate");
    let mut repository = JournalRepository::new(local);

    let snapshot_error = repository
        .append(session(), &snapshot_without_durable_message())
        .expect_err("the replacement snapshot cannot erase the partial message");
    let mut local = repository.into_inner();
    let gate_error = local
        .append(
            session(),
            DurableRecord::incremental("gate probe")
                .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session()))),
        )
        .expect_err("the rejected snapshot cannot release the physical gate");

    assert!(matches!(snapshot_error, JournalRepositoryError::Codec(_)));
    assert!(matches!(gate_error, AppendError::SnapshotRequired { .. }));
}

// semantic commit의 Session과 physical append 대상 Session이 다르면 CRC를 계산하거나
// repository를 호출하기 전에 거부해 cross-Session replay authority를 만들지 않아야 한다.
#[test]
fn rejects_a_commit_for_a_different_physical_session() {
    let other_session = crate::fixture_session(9);
    let mut repository = JournalRepository::new(RecordingRepository::default());

    let error = repository
        .append(other_session, &command_commit())
        .expect_err("semantic and physical Session identities must match");
    let inner = repository.into_inner();

    assert!(matches!(error, JournalRepositoryError::Codec(_)));
    assert!(inner.appended.is_empty());
}

// repository suffix를 semantic codec으로 복구할 때 physical envelope와 payload의
// Journal cutoff를 대조하고 원래 commit의 두 record를 같은 순서로 되살려야 한다.
#[test]
fn recovers_semantic_commits_from_repository_envelopes() {
    let commit = command_commit_at(1);
    let payload = encode(&commit).expect("the semantic commit encodes");
    let repository = RecordingRepository {
        entries: vec![RepositoryEntry::new(
            RepositorySequence::new(1),
            DurableRecord::incremental(payload).with_journal_cutoff(commit.journal_cutoff()),
        )],
        ..RecordingRepository::default()
    };
    let repository = JournalRepository::new(repository);

    let recovered = repository
        .recover(session())
        .expect("the semantic commit recovers");

    assert_eq!(recovered.records(), commit.records());
    assert!(recovered.recovery_commit().is_none());
}

// semantic payload의 Session identity가 recovery 대상과 다르면 오류가 손상된 physical
// RepositorySequence를 함께 알려 운영자가 해당 JSONL envelope를 바로 찾을 수 있어야 한다.
#[test]
fn recovery_diagnostics_include_the_physical_sequence() {
    let commit = command_commit_at(1);
    let payload = encode(&commit).expect("the semantic commit encodes");
    let repository = RecordingRepository {
        entries: vec![RepositoryEntry::new(
            RepositorySequence::new(7),
            DurableRecord::incremental(payload).with_journal_cutoff(commit.journal_cutoff()),
        )],
        ..RecordingRepository::default()
    };
    let repository = JournalRepository::new(repository);
    let other_session = crate::fixture_session(9);

    let error = repository
        .recover(other_session)
        .expect_err("cross-Session recovery is corruption");

    assert!(error.to_string().contains("repository sequence 7"));
}
