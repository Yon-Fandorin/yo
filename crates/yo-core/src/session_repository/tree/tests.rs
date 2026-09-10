use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use super::*;
use crate::{
    BackendBindingEvidence, SessionDescriptor, fixture_descriptor, fixture_session,
    journal::codec::JournalCommit,
    session_repository::{
        LocalSessionReader, LocalSessionRepository, StoredSessionReader, journal::JournalRepository,
    },
};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yo-session-tree-{}",
            WorkspaceHostId::new().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn log(&self, id: SessionId) -> PathBuf {
        self.0.join(format!("{id}.jsonl"))
    }

    fn query(&self, limits: SessionTreeLimits) -> StoredSessionTree {
        let descriptor = fixture_descriptor(fixture_session(1));
        LocalSessionReader::open(&self.0)
            .unwrap()
            .read_tree(
                descriptor.workspace_host_id(),
                descriptor.workspace_path(),
                limits,
            )
            .unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn persist(root: &Path, descriptor: SessionDescriptor, repeats: usize) {
    use crate::{
        AgentEvent, JournalSequence,
        journal::codec::{ReplaySequence, SequencedJournalRecord},
    };
    let id = descriptor.session_id();
    let repository = LocalSessionRepository::open(root, 32 * 1024 * 1024).unwrap();
    let mut journal = JournalRepository::new(repository);
    assert!((1..=2).contains(&repeats));
    journal
        .append(id, &JournalCommit::descriptor(descriptor))
        .unwrap();
    if repeats == 2 {
        journal
            .append(
                id,
                &JournalCommit::incremental_through(
                    JournalSequence::new(1),
                    vec![SequencedJournalRecord::with_journal_sequence(
                        ReplaySequence::new(2),
                        JournalSequence::new(1),
                        JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                            session_id: id,
                        }),
                    )],
                ),
            )
            .unwrap();
    }
}

fn limits(bytes: u64, records: usize) -> SessionTreeLimits {
    SessionTreeLimits::try_new(4096, 64, bytes, records).unwrap()
}

// Reader를 연 뒤 root 경로를 다른 저장소의 symlink로 바꿔도 고정한 원본만 반복 조회합니다.
#[test]
fn disk_tree_keeps_opened_root_when_its_path_is_replaced() {
    let original = Directory::new();
    let replacement = Directory::new();
    let descriptor = fixture_descriptor(fixture_session(1));
    persist(&original.0, descriptor.clone(), 1);
    persist(&replacement.0, fixture_descriptor(fixture_session(2)), 1);
    let reader = LocalSessionReader::open(&original.0).unwrap();
    let moved = Directory(original.0.with_extension("moved"));
    fs::rename(&original.0, &moved.0).unwrap();
    symlink(&replacement.0, &original.0).unwrap();
    for _ in 0..2 {
        let tree = reader
            .read_tree(
                descriptor.workspace_host_id(),
                descriptor.workspace_path(),
                SessionTreeLimits::default(),
            )
            .unwrap();
        assert_eq!(tree.nodes().len(), 1);
        assert_eq!(tree.nodes()[0].session_id(), fixture_session(1));
        assert!(!tree.truncated());
    }
    fs::remove_file(&original.0).unwrap();
}

fn exact_fork_snapshot(
    child: SessionId,
    parent: SessionId,
) -> (JournalCommit, BackendBindingEvidence) {
    use crate::{
        AgentEvent, BackendIdentity, ContextPolicyChanged, ContextStrategy, ContinuationStrategy,
        JournalSequence, ModelReplayContract, ModelReplayItem, ModelReplayRole, ReplayExecutor,
        ReplayProfile,
        journal::codec::{
            BackendBindingOpened, BindingTransition, ForkExactReplay, ForkGroup,
            ForkItemCoordinate, ForkItemOrigin, ForkSeed, ForkSource, ForkSourcePoint,
            InitialForkSeed, JournalRecord, ReplaySequence, SequencedJournalRecord,
            VersionedIdentity,
        },
    };
    let strategy = ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    };
    let binding = BackendBindingEvidence::new(
        "managed",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", child.to_string()),
        strategy,
    );
    let source_binding = BackendBindingEvidence::new(
        binding.backend_kind(),
        binding.backend_version(),
        binding.binding_identity().clone(),
        binding.model_identity().clone(),
        BackendIdentity::new("locator/v1", parent.to_string()),
        strategy,
    );
    let coordinate = ForkItemCoordinate::new(parent, 1, 1, JournalSequence::new(7), 0).unwrap();
    let replay = ForkExactReplay::new(
        ModelReplayContract::new("system", vec![]),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "parent context".into(),
            refusal: None,
        }],
        vec![ForkItemOrigin::new(coordinate, coordinate, source_binding.clone()).unwrap()],
        vec![ForkGroup::new(0, 1).unwrap()],
    )
    .unwrap();
    let records = vec![
        JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child }),
        JournalRecord::InitialForkSeed(Box::new(
            InitialForkSeed::new(
                child,
                parent,
                ForkSource::Anchor(
                    ForkSourcePoint::new(
                        1,
                        1,
                        JournalSequence::new(9),
                        JournalSequence::new(8),
                        source_binding,
                    )
                    .unwrap(),
                ),
                ForkSeed::ExactReplay(replay),
                vec![],
                2,
            )
            .unwrap(),
        )),
        JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
            1,
            "managed",
            "1",
            VersionedIdentity::new("binding/v1", "account"),
            VersionedIdentity::new("model/v1", "model"),
            VersionedIdentity::new("locator/v1", child.to_string()),
            BindingTransition::initial_fork(JournalSequence::new(2)),
            strategy,
        )),
        JournalRecord::ContextPolicyChanged(
            ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65536),
            )
            .unwrap(),
        ),
    ];
    let mut entries = vec![SequencedJournalRecord::storage(
        ReplaySequence::new(1),
        JournalRecord::SessionDescriptor(fixture_descriptor(child)),
    )];
    entries.extend(records.into_iter().enumerate().map(|(index, record)| {
        let sequence = u64::try_from(index).unwrap() + 1;
        SequencedJournalRecord::with_journal_sequence(
            ReplaySequence::new(sequence + 1),
            JournalSequence::new(sequence),
            record,
        )
    }));
    (
        JournalCommit::snapshot_through(JournalSequence::new(4), entries),
        binding,
    )
}

// 실제 child가 새 Turn을 완료해 initial-seed discovery hint가 최신 Anchor로 바뀌어도
// 계보는 전체 seed에서 찾습니다. 부모 파일 삭제는 child eligibility를 바꾸지 않습니다.
#[test]
fn disk_tree_keeps_inline_parent_after_child_work_and_parent_deletion() {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use crate::{
        AgentCommand, AgentIntent, AgentSession, BackendCommandEvidence, BackendEvent,
        BackendIdentity, BackendOutcomeEvidence, BackendRequestEvidence, BackendResumeSource,
        BackendScriptStep, CommandAdmission, JournalSequence, ModelReplayDelta, ModelReplayItem,
        ModelReplayRole, ScriptedBackend, TurnId, TurnRef, UserInput,
        journal::codec::recover,
        session_repository::{
            ContinuationEligibility, build_continuation, read_stored_session_continuation,
        },
    };
    let directory = Directory::new();
    let parent = fixture_session(1);
    let child = fixture_session(2);
    persist(&directory.0, fixture_descriptor(parent), 1);
    let (snapshot, binding) = exact_fork_snapshot(child, parent);
    let continuation = build_continuation(recover(&[snapshot]).unwrap(), child).unwrap();
    let turn = TurnRef::new(child, TurnId::new(1.try_into().unwrap()));
    let input = UserInput::new("new child work");
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(continuation.target().clone()),
            evidence: binding,
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: input.clone(),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "request/v1",
                BackendIdentity::new("exchange/v1", "one"),
                BackendIdentity::new("accepted/v1", "one"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "outcome/v1",
                "one",
            ))
            .with_replay(ModelReplayDelta::new(
                None,
                vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: input.as_str().to_owned(),
                        refusal: None,
                    },
                    ModelReplayItem::Message {
                        role: ModelReplayRole::Assistant,
                        content: "done".into(),
                        refusal: None,
                    },
                ],
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let repository = LocalSessionRepository::open(&directory.0, 32 * 1024 * 1024).unwrap();
    let mut live = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    let mut admission = live
        .dispatch(AgentIntent::submit(input.as_str()).unwrap())
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
        admission = live.retry(pending).unwrap();
    }
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    loop {
        live.poll().unwrap();
        if read_stored_session_continuation(&reader, child).is_ok_and(|continuation| {
            matches!(
                continuation.target().source(),
                Some(BackendResumeSource::ContinuationAnchor(_))
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child Anchor did not become durable"
        );
        thread::sleep(Duration::from_millis(1));
    }
    live.shutdown().unwrap();
    let before = directory.query(SessionTreeLimits::default());
    let child_node = before
        .nodes()
        .iter()
        .find(|node| node.session_id() == child)
        .unwrap();
    assert_eq!(child_node.parent_session_id(), Some(parent));
    assert_eq!(
        child_node.source(),
        Some(InheritedHistorySource::Anchor {
            record_sequence: JournalSequence::new(9),
            journal_boundary: JournalSequence::new(8),
        })
    );
    assert_eq!(child_node.depth(), 1);
    assert_eq!(
        child_node.metadata().unwrap().continuation_eligibility(),
        ContinuationEligibility::Eligible
    );
    fs::remove_file(directory.log(parent)).unwrap();
    let after = directory.query(SessionTreeLimits::default());
    assert_eq!(
        after.nodes()[0].placeholder(),
        Some(SessionTreePlaceholder::MissingAncestor)
    );
    assert_eq!(after.nodes()[1].session_id(), child);
    assert_eq!(after.nodes()[1].parent_session_id(), Some(parent));
    assert_eq!(
        after.nodes()[1]
            .metadata()
            .unwrap()
            .continuation_eligibility(),
        ContinuationEligibility::Eligible
    );
    assert!(read_stored_session_continuation(&reader, child).is_ok());
}

// 계보 없는 legacy descriptor는 root로 발명하지 않고 unknown으로 남기며, 읽기 전용 조회는
// 저장된 bytes나 coordination 파일 목록을 바꾸지 않습니다.
#[test]
fn disk_tree_keeps_legacy_ancestry_unknown_and_does_not_create_files() {
    let directory = Directory::new();
    let id = fixture_session(1);
    persist(&directory.0, fixture_descriptor(id), 1);
    let before = fs::read(directory.log(id)).unwrap();
    let mut names = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    let tree = directory.query(SessionTreeLimits::default());
    assert!(!tree.truncated());
    assert_eq!(tree.nodes().len(), 1);
    assert_eq!(
        tree.nodes()[0].ancestry(),
        &SessionTreeAncestry::UnknownLegacy
    );
    assert_eq!(tree.nodes()[0].parent_session_id(), None);
    assert_eq!(fs::read(directory.log(id)).unwrap(), before);
    let mut after = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    after.sort();
    assert_eq!(after, names);
    let missing = directory.0.join("absent");
    assert!(LocalSessionReader::open(&missing).is_err());
    assert!(!missing.exists());
}

// 실제 physical 파일의 정확한 마지막 byte/record까지는 검증하고, 첫 초과 단위부터는
// partial ancestry를 사실로 만들지 않고 uninspected 상태로 보존합니다.
#[test]
fn disk_tree_rejects_first_excess_byte_and_record() {
    let directory = Directory::new();
    let id = fixture_session(1);
    persist(&directory.0, fixture_descriptor(id), 2);
    let bytes = fs::metadata(directory.log(id)).unwrap().len();
    let exact = directory.query(limits(bytes, 2));
    assert!(!exact.truncated());
    assert_eq!(
        exact.nodes()[0].ancestry(),
        &SessionTreeAncestry::UnknownLegacy
    );
    for short in [limits(bytes - 1, 2), limits(bytes, 1)] {
        let tree = directory.query(short);
        assert!(tree.truncated());
        assert_eq!(
            tree.nodes()[0].ancestry(),
            &SessionTreeAncestry::Uninspected
        );
        assert_eq!(
            tree.nodes()[0].placeholder(),
            Some(SessionTreePlaceholder::Uninspected)
        );
        assert!(tree.nodes()[0].metadata().is_none());
    }
}

// 관련 없는 파일도 directory budget을 소비하며, 세션 수 제한 뒤의 후보를 읽어서
// 무제한 fallback을 만들지 않습니다.
#[test]
fn disk_tree_counts_irrelevant_directory_entries_and_session_candidates() {
    let directory = Directory::new();
    fs::write(directory.0.join("one.txt"), b"one").unwrap();
    fs::write(directory.0.join("two.txt"), b"two").unwrap();
    let exact = SessionTreeLimits::try_new(2, 1, 1024, 1).unwrap();
    assert!(!directory.query(exact).truncated());
    assert!(
        directory
            .query(SessionTreeLimits::try_new(1, 1, 1024, 1).unwrap())
            .truncated()
    );
    persist(&directory.0, fixture_descriptor(fixture_session(1)), 1);
    persist(&directory.0, fixture_descriptor(fixture_session(2)), 1);
    let tree = directory.query(SessionTreeLimits::try_new(4096, 1, 1024 * 1024, 10).unwrap());
    assert!(tree.truncated());
    assert_eq!(tree.nodes().len(), 1);
    assert_eq!(tree.nodes()[0].session_id(), fixture_session(1));
}

// corruption과 unsupported schema를 삭제하거나 무시하지 않고 해당 후보에 표시합니다.
// foreign workspace의 검증된 Session은 현재 workspace tree에 섞이지 않습니다.
#[test]
fn disk_tree_labels_invalid_entries_and_filters_valid_other_workspaces() {
    let directory = Directory::new();
    let corrupt = fixture_session(1);
    let unsupported = fixture_session(2);
    for (id, bytes) in [
        (corrupt, b"not-json\n".as_slice()),
        (unsupported, b"{\"schema\":\"unknown/v1\"}\n".as_slice()),
    ] {
        fs::write(directory.log(id), bytes).unwrap();
        fs::set_permissions(directory.log(id), fs::Permissions::from_mode(0o600)).unwrap();
    }
    let foreign = SessionDescriptor::new(
        WorkspaceHostId::new().unwrap(),
        HostWorkspacePath::normalize_local(&directory.0).unwrap(),
    )
    .unwrap();
    persist(&directory.0, foreign, 1);
    let tree = directory.query(SessionTreeLimits::default());
    assert_eq!(tree.nodes().len(), 2);
    for node in tree.nodes() {
        assert!(matches!(
            node.ancestry(),
            SessionTreeAncestry::Invalid { .. }
        ));
        assert_eq!(
            node.placeholder(),
            Some(SessionTreePlaceholder::Unavailable)
        );
        assert!(matches!(
            node.metadata(),
            Some(StoredSession::Unavailable { .. })
        ));
    }
}

// 마지막 envelope의 eligible hint가 맞아도 앞선 discovery 주장이 semantic prefix와 다르면
// full tree 검증은 ancestry와 eligibility를 거부하고 부모 관계를 만들어 내지 않습니다.
#[test]
fn disk_tree_validates_every_discovery_envelope_before_using_seed_parent() {
    use crate::{
        JournalSequence,
        journal::codec::encode,
        session_repository::{
            ContinuationEligibility, DurableRecord, RecordDiscovery, SessionRepository,
        },
    };
    let directory = Directory::new();
    let child = fixture_session(2);
    let descriptor = fixture_descriptor(child);
    let (snapshot, _) = exact_fork_snapshot(child, fixture_session(1));
    let mut repository = LocalSessionRepository::open(&directory.0, 32 * 1024 * 1024).unwrap();
    repository
        .append(
            child,
            DurableRecord::snapshot(encode(&snapshot).unwrap())
                .with_journal_cutoff(Some(JournalSequence::new(4)))
                .with_discovery(
                    RecordDiscovery::new(descriptor.clone())
                        .with_binding_epoch(99)
                        .with_initial_fork_seed(JournalSequence::new(2)),
                ),
        )
        .unwrap();
    repository
        .append(
            child,
            DurableRecord::snapshot(encode(&snapshot).unwrap())
                .with_journal_cutoff(Some(JournalSequence::new(4)))
                .with_discovery(
                    RecordDiscovery::new(descriptor)
                        .with_binding_epoch(1)
                        .with_initial_fork_seed(JournalSequence::new(2)),
                ),
        )
        .unwrap();
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    assert_eq!(
        reader.discover().unwrap()[0].continuation_eligibility(),
        ContinuationEligibility::Eligible
    );
    let tree = directory.query(SessionTreeLimits::default());
    assert_eq!(tree.nodes().len(), 1);
    assert!(
        matches!(tree.nodes()[0].ancestry(), SessionTreeAncestry::Invalid { detail } if detail.contains("discovery")),
        "unexpected tree ancestry: {:?}",
        tree.nodes()[0].ancestry()
    );
    assert_eq!(tree.nodes()[0].parent_session_id(), None);
    assert_eq!(
        tree.nodes()[0]
            .metadata()
            .unwrap()
            .continuation_eligibility(),
        ContinuationEligibility::Unavailable
    );
}

// FIFO와 symbolic link를 열어 대기하거나 다른 파일을 따라가지 않습니다. 잠금 관찰도
// 생성이나 blocking lock을 사용하지 않고 기존 active marker의 durable cutoff만 읽습니다.
#[test]
fn disk_tree_rejects_special_files_and_observes_active_pending_cutoff() {
    use rustix::fs::{CWD, Mode, mkfifoat};
    let directory = Directory::new();
    let id = fixture_session(1);
    persist(&directory.0, fixture_descriptor(id), 1);
    let cutoff = fs::metadata(directory.log(id)).unwrap().len();
    let writer = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.0.join(format!("{id}.writer.lock")))
        .unwrap();
    writer.try_lock().unwrap();
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(directory.log(id).with_extension("jsonl.pending"))
        .unwrap();
    marker.try_lock().unwrap();
    writeln!(marker, "{cutoff}").unwrap();
    OpenOptions::new()
        .append(true)
        .open(directory.log(id))
        .unwrap()
        .write_all(b"uncommitted tail\n")
        .unwrap();
    let active = directory.query(SessionTreeLimits::default());
    assert_eq!(
        active.nodes()[0].ancestry(),
        &SessionTreeAncestry::UnknownLegacy
    );
    let fifo = directory.log(fixture_session(2));
    mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
    symlink(directory.log(id), directory.log(fixture_session(3))).unwrap();
    let tree = directory.query(SessionTreeLimits::default());
    assert_eq!(tree.nodes().len(), 3);
    for node in &tree.nodes()[1..] {
        assert_eq!(
            node.placeholder(),
            Some(SessionTreePlaceholder::Unavailable)
        );
    }
    drop(marker);
    drop(writer);
}

fn candidate(
    id: u64,
    parent: Option<u64>,
    workspace: Option<(WorkspaceHostId, HostWorkspacePath)>,
) -> TreeCandidate {
    TreeCandidate {
        node: SessionTreeNode {
            session_id: fixture_session(id),
            metadata: None,
            depth: 0,
            ancestry: parent.map_or(SessionTreeAncestry::UnknownLegacy, |parent| {
                SessionTreeAncestry::ValidatedFork {
                    parent_session_id: fixture_session(parent),
                    source: InheritedHistorySource::Empty,
                }
            }),
            placeholder: None,
        },
        workspace,
    }
}

// 이미 검증된 collected graph만 정렬하며 cycle을 거부합니다. missing 판정과 workspace 밖
// 부모의 placeholder는 child를 숨기거나 부모의 조상을 추가 조회하지 않습니다.
#[test]
fn graph_orders_children_preserves_placeholders_and_rejects_cycles() {
    let descriptor = fixture_descriptor(fixture_session(1));
    let here = Some((
        descriptor.workspace_host_id(),
        descriptor.workspace_path().clone(),
    ));
    let other = Some((
        WorkspaceHostId::new().unwrap(),
        descriptor.workspace_path().clone(),
    ));
    let tree = assemble(
        vec![
            candidate(5, Some(4), here.clone()),
            candidate(4, None, other),
            candidate(3, Some(2), here.clone()),
            candidate(1, None, here.clone()),
        ],
        descriptor.workspace_host_id(),
        descriptor.workspace_path(),
        false,
        |id| {
            assert_eq!(id, fixture_session(2));
            SessionTreePlaceholder::MissingAncestor
        },
    );
    assert_eq!(
        tree.nodes()
            .iter()
            .map(|node| (node.session_id(), node.depth()))
            .collect::<Vec<_>>(),
        vec![
            (fixture_session(1), 0),
            (fixture_session(2), 0),
            (fixture_session(3), 1),
            (fixture_session(4), 0),
            (fixture_session(5), 1)
        ]
    );
    assert_eq!(
        tree.nodes()[1].placeholder(),
        Some(SessionTreePlaceholder::MissingAncestor)
    );
    assert_eq!(
        tree.nodes()[3].placeholder(),
        Some(SessionTreePlaceholder::OutsideWorkspace)
    );
    let cycle = assemble(
        vec![
            candidate(1, Some(2), here.clone()),
            candidate(2, Some(1), here),
        ],
        descriptor.workspace_host_id(),
        descriptor.workspace_path(),
        false,
        |_| panic!("collected parents need no read"),
    );
    assert_eq!(cycle.nodes().len(), 2);
    assert!(
        cycle
            .nodes()
            .iter()
            .all(|node| matches!(node.ancestry(), SessionTreeAncestry::Invalid { .. }))
    );
}

// default reader에게 bounded operation이 없으면 discover/read_session을 대신 호출하지 않습니다.
#[test]
fn default_tree_operation_and_limits_fail_closed() {
    use crate::session_repository::{RepositorySequence, StoredSessionSnapshot};
    struct Unsupported;
    impl StoredSessionReader for Unsupported {
        fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
            panic!("no discovery fallback")
        }
        fn read_session(&self, _: SessionId) -> Result<StoredSessionSnapshot, RepositoryError> {
            panic!("no snapshot fallback")
        }
        fn read_after(
            &self,
            _: SessionId,
            _: Option<RepositorySequence>,
            _: usize,
        ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
            panic!("no history fallback")
        }
    }
    let descriptor = fixture_descriptor(fixture_session(1));
    let reader: Box<dyn StoredSessionReader> = Box::new(Unsupported);
    assert!(
        reader
            .read_tree(
                descriptor.workspace_host_id(),
                descriptor.workspace_path(),
                SessionTreeLimits::default()
            )
            .is_err()
    );
    for limits in [
        (0, 1, 1, 1),
        (1, 0, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1, 0),
        (1, 1025, 1, 1),
    ] {
        assert!(SessionTreeLimits::try_new(limits.0, limits.1, limits.2, limits.3).is_err());
    }
}
