use super::{
    super::{JournalCommit, JournalRecord, activity, encode, recover, submission},
    support::{identity, semantic, valid_history},
};
use crate::{
    AgentCommand, AgentEvent, ContinuationStrategy, JournalSequence, ReplayExecutor, TurnOutcome,
    TurnRef,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, BindingCloseReason, BindingTransition,
        CacheState, ContinuationAnchor, DetailAvailability, ExchangeDirection, ExchangeKind,
        OperationId, TransitionMode,
    },
};

fn replacement_commit(
    epoch: u64,
    source_anchor: u64,
    first_replay: u64,
    first_journal: u64,
) -> JournalCommit {
    JournalCommit::incremental_through(
        JournalSequence::new(first_journal + 1),
        vec![
            semantic(
                first_replay,
                first_journal,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    epoch - 1,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                first_replay + 1,
                first_journal + 1,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    epoch,
                    "codex",
                    "1.0.0",
                    identity(&format!("binding-{epoch}")),
                    identity(&format!("model-{epoch}")),
                    identity(&format!("session-{epoch}")),
                    BindingTransition::new(
                        TransitionMode::ExactReplay,
                        CacheState::Lost,
                        Some(JournalSequence::new(source_anchor)),
                    ),
                    ContinuationStrategy::BackendManagedState,
                )),
            ),
        ],
    )
}

fn local_exact_replay_replacement_commit(
    epoch: u64,
    first_replay: u64,
    first_journal: u64,
) -> JournalCommit {
    JournalCommit::incremental_through(
        JournalSequence::new(first_journal + 1),
        vec![
            semantic(
                first_replay,
                first_journal,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    epoch - 1,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                first_replay + 1,
                first_journal + 1,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    epoch,
                    "native",
                    "1.0.0",
                    identity(&format!("binding-{epoch}")),
                    identity(&format!("model-{epoch}")),
                    identity(&format!("session-{epoch}")),
                    BindingTransition::new(
                        TransitionMode::ExactReplay,
                        CacheState::Lost,
                        Some(JournalSequence::new(9)),
                    ),
                    ContinuationStrategy::ExactReplay {
                        executor: ReplayExecutor::LocalClient,
                        replay_profile: crate::ReplayProfile::SemanticOnly,
                    },
                )),
            ),
        ],
    )
}

fn backend_managed_replacement_commit() -> JournalCommit {
    JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![
            semantic(
                11,
                10,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    1,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                12,
                11,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    2,
                    "server",
                    "1.0.0",
                    identity("binding-2"),
                    identity("model-2"),
                    identity("session-2"),
                    BindingTransition::new(
                        TransitionMode::LossyHandoff,
                        CacheState::Lost,
                        Some(JournalSequence::new(9)),
                    ),
                    ContinuationStrategy::BackendManagedState,
                )),
            ),
        ],
    )
}

fn completed_turn_in_epoch_two() -> Vec<JournalCommit> {
    let submission_id = submission(13);
    let operation_id = OperationId::from(submission_id);
    let turn = TurnRef::new(
        activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(4).unwrap()),
    );
    vec![
        JournalCommit::incremental_through(
            JournalSequence::new(14),
            vec![
                semantic(
                    13,
                    12,
                    JournalRecord::CommandCommitted(
                        crate::journal::CommittedCommand::submission(
                            AgentCommand::StartTurn {
                                turn,
                                input: crate::UserInput::new("epoch two"),
                            },
                            submission_id,
                        )
                        .unwrap(),
                    ),
                ),
                semantic(
                    14,
                    13,
                    JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                        2,
                        operation_id,
                        ExchangeKind::Request,
                        ExchangeDirection::YoToBackend,
                        "codex.request/v1",
                        None,
                        None,
                        DetailAvailability::Unpersisted,
                    )),
                ),
                semantic(
                    15,
                    14,
                    JournalRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                        2,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(13),
                        identity("request-2"),
                    )),
                ),
            ],
        ),
        JournalCommit::incremental_through(
            JournalSequence::new(17),
            vec![
                semantic(
                    16,
                    15,
                    JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                        turn,
                        outcome: TurnOutcome::Completed,
                    }),
                ),
                semantic(
                    17,
                    16,
                    JournalRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                        2,
                        turn.turn_id(),
                        JournalSequence::new(14),
                        Some(identity("outcome-2")),
                        None,
                    )),
                ),
                semantic(
                    18,
                    17,
                    JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
                        2,
                        JournalSequence::new(14),
                        JournalSequence::new(16),
                        JournalSequence::new(16),
                    )),
                ),
            ],
        ),
    ]
}

// 완결 Anchor 뒤에 새 semantic command와 outbound request가 생기면 그 suffix는 아직
// 재개 경계로 완결되지 않았으므로 이전 Anchor를 discovery 후보로 계속 내보내면 안 됩니다.
#[test]
fn clears_the_latest_anchor_when_a_new_semantic_suffix_begins() {
    let mut commits = valid_history();
    let submission_id = submission(12);
    let turn = crate::TurnRef::new(
        activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(4).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![
            semantic(
                11,
                10,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("next"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                12,
                11,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    OperationId::from(submission_id),
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "codex.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
        ],
    ));

    let recovered = recover(&commits).expect("the unfinished next request remains valid history");

    assert_eq!(recovered.binding_epoch(), Some(1));
    assert_eq!(recovered.continuation_anchor(), None);
}

// revoked나 exhausted로 binding이 닫히면 과거 Anchor의 record는 replacement 검증을 위해
// 남더라도 현재 native resume 후보는 아니므로 discovery Anchor는 비워야 합니다.
#[test]
fn clears_the_latest_anchor_when_the_binding_closes() {
    let mut commits = valid_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(10),
        vec![semantic(
            11,
            10,
            JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                1,
                BindingCloseReason::Revoked,
            )),
        )],
    ));

    let recovered = recover(&commits).expect("the revoked binding is valid closed history");

    assert_eq!(recovered.binding_epoch(), None);
    assert_eq!(recovered.continuation_anchor(), None);
}

// epoch 1을 replaced로 닫은 직후 그 epoch의 최신 Anchor를 source로 쓰면 epoch 2가
// 정상적으로 열립니다. Backend-managed binding은 이전 Anchor를 현재 resume 후보로
// 노출하지 않습니다.
#[test]
fn opens_the_next_epoch_from_the_immediately_preceding_anchor() {
    let mut commits = valid_history();
    commits.push(replacement_commit(2, 9, 11, 10));

    let recovered = recover(&commits).expect("the direct replacement lineage is valid");

    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.continuation_anchor(), None);
}

// Local exact replay replacement는 새 epoch에서 아직 Turn이 없더라도 durable source
// Anchor와 누적 replay를 사용해 즉시 다시 열 수 있어야 합니다.
#[test]
fn local_exact_replay_replacement_keeps_its_source_as_the_resume_anchor() {
    let mut commits = valid_history();
    commits.push(local_exact_replay_replacement_commit(2, 11, 10));

    let recovered = recover(&commits).expect("the local exact replay lineage is valid");

    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(
        recovered.continuation_anchor(),
        Some(JournalSequence::new(9))
    );
}

// 새 Turn 없이 local exact-replay binding을 연속 교체하면 바로 전 epoch가 물려받은
// 동일 source Anchor를 다시 전달해도 계보가 이어져야 합니다.
#[test]
fn consecutive_local_exact_replay_replacements_keep_the_inherited_anchor() {
    let mut commits = valid_history();
    commits.push(local_exact_replay_replacement_commit(2, 11, 10));
    commits.push(local_exact_replay_replacement_commit(3, 13, 12));

    let recovered = recover(&commits).expect("the inherited local replay Anchor is valid");

    assert_eq!(recovered.binding_epoch(), Some(3));
    assert_eq!(
        recovered.continuation_anchor(),
        Some(JournalSequence::new(9))
    );
}

// exact replay epoch를 backend-managed binding으로 교체하면 이전 Yo replay는 새 Anchor가
// 생긴 뒤에도 resume target 후보에 남지 않아야 한다.
#[test]
fn backend_managed_replacement_clears_the_previous_exact_replay() {
    let mut commits = valid_history();
    commits.push(backend_managed_replacement_commit());
    commits.extend(completed_turn_in_epoch_two());

    let recovered = recover(&commits).expect("the backend-managed replacement is valid");

    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(
        recovered.continuation_anchor(),
        Some(JournalSequence::new(17))
    );
    assert_eq!(recovered.model_replay(), &crate::ModelReplay::default());
}

// epoch 2에도 새 Anchor가 생긴 뒤 epoch 3이 epoch 1의 오래된 Anchor로 되돌아가면 최신
// 불완결 흐름을 건너뛸 수 있으므로 바로 이전 epoch의 source가 아니라고 거부해야 합니다.
#[test]
fn rejects_a_replacement_that_falls_back_to_an_older_epoch_anchor() {
    let mut commits = valid_history();
    commits.push(replacement_commit(2, 9, 11, 10));
    commits.extend(completed_turn_in_epoch_two());
    commits.push(replacement_commit(3, 9, 19, 18));

    let error = recover(&commits).expect_err("epoch three cannot cite epoch one's stale Anchor");

    assert!(error.to_string().contains("immediately preceding epoch"));
}

// Anchor가 outcome의 JournalSequence 대신 자기 번호를 boundary로 쓰면 순환적인 완료
// 주장이 되므로 encode 단계에서 즉시 거부해야 저장소에 잘못된 재개 지점이 들어가지 않습니다.
#[test]
fn rejects_an_anchor_that_claims_its_own_sequence_as_the_boundary() {
    let mut commits = valid_history();
    let completion = commits.pop().unwrap();
    let mut records = completion.records().to_vec();
    records[3] = semantic(
        10,
        9,
        JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
            1,
            JournalSequence::new(5),
            JournalSequence::new(8),
            JournalSequence::new(9),
        )),
    );
    let invalid = JournalCommit::incremental_through(JournalSequence::new(9), records);

    let error = encode(&invalid).expect_err("an Anchor cannot claim itself as its boundary");

    assert!(error.to_string().contains("boundary"));
}
