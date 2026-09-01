use sha2::{Digest, Sha256};

use super::{
    super::{decode, encode, recover},
    support::{identity, semantic},
};
use crate::{
    AgentCommand, AgentEvent, ContinuationStrategy, JournalSequence, ModelReplayContract,
    ModelReplayDelta, ModelReplayItem, ModelReplayRole, ProviderPrivateReplayEnvelope,
    ReplayExecutor, ReplayProfile, TurnOutcome,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, BindingCloseReason, BindingTransition,
        CacheState, ContextArtifactReceipt, ContextCheckpoint, ContextLoss, ContextPolicyChanged,
        ContextRetainedGroup, ContextStrategy, ContextSummaryUsage, ContinuationAnchor,
        DetailAvailability, ExchangeDirection, ExchangeKind, JournalCommit, JournalRecord,
        MessageEnded, MessageOutcome, MessageStream, MessageTerminal, ModelReplayDeltaRecord,
        OperationId, ReplaySequence, SequencedJournalRecord, TransitionMode,
    },
};

fn portable_body() -> &'static str {
    "# Context Checkpoint\n\
## Current Objective\nContinue the task.\n\
## Active Constraints\nNone.\n\
## Decisions\nUse a durable checkpoint.\n\
## Verified Progress\nThe first request completed.\n\
## Current State\nThe Session is idle.\n\
## Unknown or Unverified\nNone.\n\
## Next Actions\nResume from the checkpoint.\n\
## Critical References\nNone."
}

fn summary_usage() -> ContextSummaryUsage {
    ContextSummaryUsage::try_new(serde_json::json!({
        "schema": "yo.model-usage-receipt/v1",
        "response_id": "summary-1",
        "round": 1,
        "provider": "test",
        "account": "default",
        "model": "test-model",
        "connector": "openai-responses",
        "api_dialect": "openai-responses",
        "base_url": "https://example.invalid/",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 20,
            "total_tokens": 120,
            "reasoning_tokens": 0
        },
        "cache_read_input_tokens": { "availability": "unsupported" }
    }))
    .unwrap()
}

// summary usage는 exact source attribution과 네 token 값·관계를 모두 보존해야 하므로
// 누락·모순·상한 초과·input을 넘는 cache-read 표기를 생성 단계에서 거부합니다.
#[test]
fn rejects_incomplete_or_inconsistent_summary_usage_receipts() {
    let mut missing = summary_usage().value().clone();
    missing["usage"]
        .as_object_mut()
        .unwrap()
        .remove("input_tokens");
    assert!(ContextSummaryUsage::try_new(missing).is_err());

    let mut inconsistent = summary_usage().value().clone();
    inconsistent["usage"]["total_tokens"] = serde_json::json!(119);
    assert!(ContextSummaryUsage::try_new(inconsistent).is_err());

    let mut oversized_source = summary_usage().value().clone();
    oversized_source["response_id"] = serde_json::json!("r".repeat(257));
    assert!(ContextSummaryUsage::try_new(oversized_source).is_err());

    let mut oversized_cache = summary_usage().value().clone();
    oversized_cache["cache_read_input_tokens"] = serde_json::json!({
        "availability": "reported",
        "tokens": 101,
        "source_profile": "test-cache/v1"
    });
    assert!(ContextSummaryUsage::try_new(oversized_cache).is_err());
}

fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn policy() -> ContextPolicyChanged {
    ContextPolicyChanged::try_new(
        1,
        true,
        ContextStrategy::PortableSummaryV1Alpha1,
        85,
        90,
        Some(10),
        Some(65_536),
    )
    .unwrap()
}

fn current_history() -> Vec<JournalCommit> {
    current_history_with(
        ReplayProfile::SemanticOnly,
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "done".to_owned(),
            refusal: None,
        }],
    )
}

fn current_history_with(
    replay_profile: ReplayProfile,
    replay_items: Vec<ModelReplayItem>,
) -> Vec<JournalCommit> {
    let descriptor =
        JournalCommit::descriptor(super::super::descriptor_with_path(b"/workspace".to_vec()));
    let opened = JournalCommit::incremental_through(
        JournalSequence::new(3),
        vec![
            semantic(
                2,
                1,
                JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                    session_id: super::super::activity().session_id(),
                }),
            ),
            semantic(
                3,
                2,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    1,
                    "managed",
                    "1.0.0",
                    identity("binding"),
                    identity("model"),
                    identity("session"),
                    BindingTransition::new(
                        TransitionMode::Initial,
                        CacheState::NotApplicable,
                        None,
                    ),
                    ContinuationStrategy::ExactReplay {
                        executor: ReplayExecutor::LocalClient,
                        replay_profile,
                    },
                )),
            ),
            semantic(4, 3, JournalRecord::ContextPolicyChanged(policy())),
        ],
    );
    let submission_id = super::super::submission(42);
    let operation_id = OperationId::from(submission_id);
    let request = JournalCommit::incremental_through(
        JournalSequence::new(6),
        vec![
            semantic(
                5,
                4,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn: super::super::activity().turn(),
                            input: crate::UserInput::new("continue"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                6,
                5,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                7,
                6,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(5),
                        identity("request"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    );
    let replay = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        replay_items,
    );
    let completed = JournalCommit::incremental_through(
        JournalSequence::new(10),
        vec![
            semantic(
                8,
                7,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn: super::super::activity().turn(),
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                9,
                8,
                JournalRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        1,
                        super::super::activity().turn_id(),
                        JournalSequence::new(6),
                        replay,
                    )
                    .with_context_epoch(1),
                ),
            ),
            semantic(
                10,
                9,
                JournalRecord::BackendResumableOutcome(
                    BackendResumableOutcome::new(
                        1,
                        super::super::activity().turn_id(),
                        JournalSequence::new(6),
                        Some(identity("outcome")),
                        Some(JournalSequence::new(8)),
                    )
                    .with_context_epoch(1),
                ),
            ),
            semantic(
                11,
                10,
                JournalRecord::ContinuationAnchor(
                    ContinuationAnchor::new(
                        1,
                        JournalSequence::new(6),
                        JournalSequence::new(9),
                        JournalSequence::new(9),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    );
    vec![descriptor, opened, request, completed]
}

// 한 Turn의 첫 provider request가 이미 durable하게 수락된 뒤 도구 결과로 이어지는
// successor request는 SubmissionId가 없으므로 writer가 정한 deterministic identity로
// 복구되어야 한다.
#[test]
fn accepts_a_writer_assigned_internal_successor_request() {
    let mut commits = current_history();
    commits.pop();
    let exchange_sequence = JournalSequence::new(7);
    let operation_id = OperationId::for_internal_request(
        super::super::activity().session_id(),
        super::super::activity().turn_id(),
        exchange_sequence,
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(8),
        vec![
            semantic(
                8,
                7,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                9,
                8,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::activity().turn_id(),
                        operation_id,
                        exchange_sequence,
                        identity("successor-request"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));

    recover(&commits).unwrap();
}

// 동일한 deterministic identity라도 완료된 Turn에 사후 요청을 붙일 수는 없다.
#[test]
fn rejects_a_writer_assigned_request_after_turn_completion() {
    let mut commits = current_history();
    let exchange_sequence = JournalSequence::new(11);
    let operation_id = OperationId::for_internal_request(
        super::super::activity().session_id(),
        super::super::activity().turn_id(),
        exchange_sequence,
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(
                12,
                11,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                13,
                12,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::activity().turn_id(),
                        operation_id,
                        exchange_sequence,
                        identity("late-request"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("writer-assigned successor"));
}

// 새 Turn의 첫 request 전에 압축이 끝나는 round-zero 경로는 checkpoint가 command를
// durable하게 가로지른 경우에만 writer-assigned 첫 request를 허용한다.
#[test]
fn accepts_the_first_writer_assigned_request_after_an_active_checkpoint() {
    let mut commits = current_history();
    let second_turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
    let submission_id = super::super::submission(43);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn: second_turn,
                            input: crate::UserInput::new("round zero"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                13,
                12,
                JournalRecord::EventCommitted(AgentEvent::TurnStarted { turn: second_turn }),
            ),
        ],
    ));
    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(11),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "round zero".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![semantic(
            14,
            13,
            JournalRecord::ContextCheckpoint(checkpoint_for(
                CheckpointLineage {
                    epoch: 1,
                    previous_context_epoch: 1,
                    successor_context_epoch: 2,
                    source_anchor_sequence: JournalSequence::new(10),
                    source_journal_boundary: JournalSequence::new(11),
                },
                ModelReplayContract::new("system", Vec::new()),
                vec![retained],
                Vec::new(),
                vec![
                    ContextLoss::visible_prefix_summarized(
                        JournalSequence::new(7),
                        JournalSequence::new(9),
                    )
                    .unwrap(),
                ],
            )),
        )],
    ));
    let exchange_sequence = JournalSequence::new(14);
    let operation_id = OperationId::for_internal_request(
        second_turn.session_id(),
        second_turn.turn_id(),
        exchange_sequence,
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(15),
        vec![
            semantic(
                15,
                14,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        second_turn.turn_id(),
                        operation_id,
                        exchange_sequence,
                        identity("round-zero-request"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(2));
}

fn checkpoint() -> ContextCheckpoint {
    checkpoint_with(
        ModelReplayContract::new("system", Vec::new()),
        vec![
            ContextRetainedGroup::try_new(
                JournalSequence::new(7),
                JournalSequence::new(9),
                vec![ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: "done".to_owned(),
                    refusal: None,
                }],
            )
            .unwrap(),
        ],
        Vec::new(),
        Vec::new(),
        JournalSequence::new(10),
    )
}

fn checkpoint_with(
    replay_contract: ModelReplayContract,
    retained_groups: Vec<ContextRetainedGroup>,
    artifact_receipts: Vec<ContextArtifactReceipt>,
    losses: Vec<ContextLoss>,
    source_journal_boundary: JournalSequence,
) -> ContextCheckpoint {
    checkpoint_for(
        CheckpointLineage {
            epoch: 1,
            previous_context_epoch: 1,
            successor_context_epoch: 2,
            source_anchor_sequence: JournalSequence::new(10),
            source_journal_boundary,
        },
        replay_contract,
        retained_groups,
        artifact_receipts,
        losses,
    )
}

struct CheckpointLineage {
    epoch: u64,
    previous_context_epoch: u64,
    successor_context_epoch: u64,
    source_anchor_sequence: JournalSequence,
    source_journal_boundary: JournalSequence,
}

fn checkpoint_for(
    lineage: CheckpointLineage,
    replay_contract: ModelReplayContract,
    retained_groups: Vec<ContextRetainedGroup>,
    artifact_receipts: Vec<ContextArtifactReceipt>,
    losses: Vec<ContextLoss>,
) -> ContextCheckpoint {
    ContextCheckpoint::try_new(
        lineage.epoch,
        lineage.previous_context_epoch,
        lineage.successor_context_epoch,
        lineage.source_anchor_sequence,
        lineage.source_journal_boundary,
        1,
        ContextStrategy::PortableSummaryV1Alpha1,
        100_000,
        90_000,
        20_000,
        replay_contract,
        portable_body(),
        retained_groups.clone(),
        retained_groups
            .first()
            .map(ContextRetainedGroup::first_sequence),
        artifact_receipts,
        losses,
        summary_usage(),
    )
    .unwrap()
}

fn checkpoint_replacement_history(strategy: ContinuationStrategy) -> Vec<JournalCommit> {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let transition = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_checkpoint_sequence(JournalSequence::new(11));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![
            semantic(
                13,
                12,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    1,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                14,
                13,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    2,
                    "managed",
                    "1.0.1",
                    identity("binding-2"),
                    identity("model"),
                    identity("session"),
                    transition,
                    strategy,
                )),
            ),
        ],
    ));
    commits
}

// 새 정책·epoch·checkpoint를 물리 wire로 왕복한 뒤에도 summary와 retained group만으로
// 정확한 successor replay root와 checkpoint-only 실행 대상을 복구해야 합니다.
#[test]
fn round_trips_and_recovers_a_checkpoint_as_the_new_replay_root() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let commits = commits
        .into_iter()
        .map(|commit| decode(&encode(&commit).unwrap()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();

    assert_eq!(recovered.context_epoch(), Some(2));
    assert_eq!(
        recovered.context_checkpoint(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(recovered.continuation_anchor(), None);
    assert_eq!(recovered.model_replay().items().len(), 2);
    assert!(matches!(
        &recovered.model_replay().items()[0],
        ModelReplayItem::Message { role: ModelReplayRole::User, content, .. }
            if content == portable_body()
    ));
    let continuation = crate::session_repository::build_continuation(
        recovered,
        super::super::activity().session_id(),
    )
    .expect("a checkpoint without a later request is executable");
    assert_eq!(
        continuation.target().source_checkpoint_sequence(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(continuation.target().source_anchor_sequence(), None);
}

// checkpoint publish와 다음 dispatch 근거가 같은 physical append에 묶이면 checkpoint가
// 먼저 durable했다는 경계를 증명할 수 없으므로 checkpoint는 incremental commit의 끝입니다.
#[test]
fn rejects_records_after_a_checkpoint_in_the_same_incremental_commit() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![
            semantic(12, 11, JournalRecord::ContextCheckpoint(checkpoint())),
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("after checkpoint"),
                        },
                        super::super::submission(43),
                    )
                    .unwrap(),
                ),
            ),
        ],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("final record"));
}

// 첫 checkpoint root도 successor context의 durable source group으로 남아야 다음
// checkpoint가 이전 synthetic body를 loss로 선언하고 새 suffix만 retain할 수 있습니다.
#[test]
fn repeated_checkpoint_accounts_for_the_prior_checkpoint_root() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    let submission_id = super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("second request"),
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
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                15,
                14,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(13),
                        identity("request-2"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(18),
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
                JournalRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        1,
                        turn.turn_id(),
                        JournalSequence::new(14),
                        ModelReplayDelta::new(
                            None,
                            vec![ModelReplayItem::Message {
                                role: ModelReplayRole::Assistant,
                                content: "second answer".to_owned(),
                                refusal: None,
                            }],
                        ),
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                18,
                17,
                JournalRecord::BackendResumableOutcome(
                    BackendResumableOutcome::new(
                        1,
                        turn.turn_id(),
                        JournalSequence::new(14),
                        Some(identity("outcome-2")),
                        Some(JournalSequence::new(16)),
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                19,
                18,
                JournalRecord::ContinuationAnchor(
                    ContinuationAnchor::new(
                        1,
                        JournalSequence::new(14),
                        JournalSequence::new(17),
                        JournalSequence::new(17),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(15),
        JournalSequence::new(17),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "second answer".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let prior_root =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(11), JournalSequence::new(11))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(19),
        vec![semantic(
            20,
            19,
            JournalRecord::ContextCheckpoint(checkpoint_for(
                CheckpointLineage {
                    epoch: 1,
                    previous_context_epoch: 2,
                    successor_context_epoch: 3,
                    source_anchor_sequence: JournalSequence::new(18),
                    source_journal_boundary: JournalSequence::new(18),
                },
                ModelReplayContract::new("system", Vec::new()),
                vec![retained],
                Vec::new(),
                vec![prior_root],
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(3));
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message { content, .. }) if content == "second answer"
    ));
}

// checkpoint inline replay는 writer가 새로 꾸밀 수 있는 payload가 아니라 source
// Journal의 완료 replay group과 byte-for-byte 같아야 하므로 invented item을 거부합니다.
#[test]
fn rejects_a_checkpoint_that_invents_retained_replay() {
    let invented = ContextRetainedGroup::try_new(
        JournalSequence::new(7),
        JournalSequence::new(9),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "invented".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![invented],
                Vec::new(),
                Vec::new(),
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("exact Journal-backed replay"));
}

// retained하지 않은 완료 replay group은 visible loss로 정확히 선언해야 하므로 빈 loss
// 목록으로 source prefix를 조용히 버리는 checkpoint를 거부합니다.
#[test]
fn rejects_a_checkpoint_that_silently_omits_a_source_group() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("silently omits"));
}

// active Turn의 committed current input은 이전 Anchor 뒤 mandatory suffix이므로 source
// range와 exact user replay item을 함께 retained group에 넣은 checkpoint만 허용합니다.
#[test]
fn retains_the_complete_active_suffix_and_current_input() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("current input"),
                    },
                    super::super::submission(43),
                )
                .unwrap(),
            ),
        )],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(11),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "current input".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![semantic(
            13,
            12,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(11),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        }) if content == "current input"
    ));
}

// accepted request 뒤 도구 call/result Activity가 모두 닫힌 범위는 current input과 완전한
// call/output replay를 함께 inline 보존할 때만 실행 가능한 checkpoint source가 됩니다.
#[test]
fn retains_a_completed_post_tool_active_suffix() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    let call_activity = crate::ActivityRef::new(
        turn,
        crate::ActivityId::new(std::num::NonZeroU64::new(1).unwrap()),
    );
    let result_activity = crate::ActivityRef::new(
        turn,
        crate::ActivityId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
    let submission_id = super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(17),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("current input"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                13,
                12,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                14,
                13,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(12),
                        identity("request-2"),
                    )
                    .with_context_epoch(1),
                ),
            ),
            semantic(
                15,
                14,
                JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                    activity: call_activity,
                    kind: crate::ActivityKind::ToolCall,
                }),
            ),
            SequencedJournalRecord::storage(
                ReplaySequence::new(16),
                JournalRecord::MessageEnded(MessageTerminal::new(
                    None,
                    MessageEnded::new(
                        call_activity,
                        MessageStream::ToolOutput,
                        MessageOutcome::Completed,
                        0,
                        0,
                    ),
                )),
            ),
            semantic(
                17,
                15,
                JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                    activity: call_activity,
                    outcome: crate::ActivityOutcome::Completed,
                }),
            ),
            semantic(
                18,
                16,
                JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
                    activity: result_activity,
                    kind: crate::ActivityKind::ToolResult,
                }),
            ),
            SequencedJournalRecord::storage(
                ReplaySequence::new(19),
                JournalRecord::MessageEnded(MessageTerminal::new(
                    None,
                    MessageEnded::new(
                        result_activity,
                        MessageStream::ToolOutput,
                        MessageOutcome::Completed,
                        0,
                        0,
                    ),
                )),
            ),
            semantic(
                20,
                17,
                JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
                    activity: result_activity,
                    outcome: crate::ActivityOutcome::Completed,
                }),
            ),
        ],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(17),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "current input".to_owned(),
                refusal: None,
            },
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "workspace contents".to_owned(),
            },
        ],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(18),
        vec![semantic(
            21,
            18,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(17),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert!(matches!(
        recovered.model_replay().items().last(),
        Some(ModelReplayItem::FunctionCallOutput { call_id, output })
            if call_id == "call-1" && output == "workspace contents"
    ));
}

// Anchor 뒤 active suffix는 단순 range와 input 포함 여부만 맞추는 payload가 아니라
// 그 한 submitted input의 exact replay 표현이어야 하므로 추가 item을 끼울 수 없습니다.
#[test]
fn rejects_an_active_suffix_that_invents_replay_items() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("current input"),
                    },
                    super::super::submission(43),
                )
                .unwrap(),
            ),
        )],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(11),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "current input".to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "invented".to_owned(),
                refusal: None,
            },
        ],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![semantic(
            13,
            12,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(11),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("submitted input"));
}

// accepted request는 completed Turn/delta/outcome/Anchor가 생기기 전에는 uncertain 상태이므로
// active retained range로 감싸 executable checkpoint root로 바꿀 수 없습니다.
#[test]
fn rejects_a_checkpoint_that_cuts_through_an_accepted_request() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    let submission_id = super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(13),
        vec![
            semantic(
                12,
                11,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: crate::UserInput::new("current input"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                13,
                12,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                14,
                13,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        turn.turn_id(),
                        operation_id,
                        JournalSequence::new(12),
                        identity("request-2"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));
    let active = ContextRetainedGroup::try_new(
        JournalSequence::new(11),
        JournalSequence::new(13),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "current input".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![semantic(
            15,
            14,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![active],
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(13),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("submitted input"));
}

// active Anchor 뒤 semantic suffix를 loss로 돌리거나 retained range에서 빼면 현재 입력을
// 조용히 잃으므로 checkpoint recovery가 fail closed해야 합니다.
#[test]
fn rejects_a_checkpoint_that_omits_the_active_suffix() {
    let mut commits = current_history();
    let turn = crate::TurnRef::new(
        super::super::activity().session_id(),
        crate::TurnId::new(std::num::NonZeroU64::new(3).unwrap()),
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::CommandCommitted(
                crate::journal::CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("current input"),
                    },
                    super::super::submission(43),
                )
                .unwrap(),
            ),
        )],
    ));
    let old_prefix =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(12),
        vec![semantic(
            13,
            12,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![old_prefix],
                JournalSequence::new(11),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("active semantic suffix"));
}

// artifact disclosure는 summarized replay의 실제 visible bytes와 hash·byte count가
// 일치해야 하므로 같은 delta 좌표에 임의 hash를 붙인 receipt를 거부합니다.
#[test]
fn rejects_an_artifact_receipt_not_bound_to_visible_source_bytes() {
    let receipt = ContextArtifactReceipt::try_new(
        content_hash(b"different"),
        9,
        "text/plain",
        1,
        JournalSequence::new(8),
    )
    .unwrap();
    let loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                vec![receipt],
                vec![loss],
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("artifact receipt"));
}

// artifact receipt는 summarized replay group의 exact tool output bytes와 canonical text
// media kind에만 결속되며 ordinary assistant text는 같은 hash여도 source가 아닙니다.
#[test]
fn accepts_only_exact_summarized_tool_output_artifacts() {
    let output = "large tool output";
    let mut commits = current_history_with(
        ReplayProfile::SemanticOnly,
        vec![
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "inspect".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: output.to_owned(),
            },
        ],
    );
    let receipt = ContextArtifactReceipt::try_new(
        content_hash(output.as_bytes()),
        u64::try_from(output.len()).unwrap(),
        "text/plain",
        1,
        JournalSequence::new(8),
    )
    .unwrap();
    let loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                vec![receipt],
                vec![loss],
                JournalSequence::new(10),
            )),
        )],
    ));
    recover(&commits).unwrap();

    let assistant_receipt = ContextArtifactReceipt::try_new(
        content_hash(b"done"),
        4,
        "text/plain",
        1,
        JournalSequence::new(8),
    )
    .unwrap();
    let mut assistant_history = current_history();
    let loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    assistant_history.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                vec![assistant_receipt],
                vec![loss],
                JournalSequence::new(10),
            )),
        )],
    ));
    let error = recover(&assistant_history).unwrap_err();
    assert!(error.to_string().contains("artifact receipt"));
}

// summarized provider-private item은 source delta의 exact schema·payload byte count로 loss를
// 하나씩 공개해야 하므로 visible loss만 기록한 checkpoint를 거부합니다.
#[test]
fn rejects_missing_provider_private_loss_disclosure() {
    let private =
        ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", b"{}".to_vec())
            .unwrap();
    let mut commits = current_history_with(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "done".to_owned(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant { envelope: private },
        ],
    );
    let visible_loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![visible_loss],
                JournalSequence::new(10),
            )),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("loss disclosure"));
}

// exact private loss가 visible summarized group과 일치하면 synthetic body만 남은 root도
// private-profile source checkpoint로 안전하게 복구됩니다.
#[test]
fn accepts_exact_provider_private_loss_disclosure() {
    let private =
        ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", b"{}".to_vec())
            .unwrap();
    let mut commits = current_history_with(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "done".to_owned(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant { envelope: private },
        ],
    );
    let visible_loss =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(7), JournalSequence::new(9))
            .unwrap();
    let private_loss = ContextLoss::provider_private_dropped(
        "kimi.assistant-message/v1alpha1",
        2,
        JournalSequence::new(8),
    )
    .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![visible_loss, private_loss],
                JournalSequence::new(10),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.model_replay().items().len(), 1);
}

// source boundary는 Anchor가 commit한 outcome 경계부터 checkpoint 직전 semantic
// sequence 사이의 실제 cut을 허용하므로 Anchor outcome에서 자르는 checkpoint도 복구합니다.
#[test]
fn accepts_a_checkpoint_boundary_at_the_anchor_outcome() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                Vec::new(),
                Vec::new(),
                vec![
                    ContextLoss::visible_prefix_summarized(
                        JournalSequence::new(7),
                        JournalSequence::new(9),
                    )
                    .unwrap(),
                ],
                JournalSequence::new(9),
            )),
        )],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(2));
}

// context_epoch이 없던 legacy 요청·Anchor 뒤에 새 정책만 덧붙이면 두 형식의 의미를
// 안전하게 구분할 수 없으므로 current reader가 mixed graph를 거부해야 합니다.
#[test]
fn rejects_a_policy_added_after_legacy_context_records() {
    let mut commits = super::support::valid_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(10),
        vec![semantic(
            11,
            10,
            JournalRecord::ContextPolicyChanged(policy()),
        )],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("legacy context graph"));
}

// checkpoint가 epoch 2를 연 뒤 후속 accepted request가 superseded epoch 1을 다시 쓰면
// 과거 증거와 새 model context가 교차하므로 복구 단계에서 거부해야 합니다.
#[test]
fn rejects_a_stale_context_epoch_after_a_checkpoint() {
    let mut commits = current_history();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint()),
        )],
    ));
    let submission_id = super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(14),
        vec![
            semantic(
                13,
                12,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn: super::super::activity().turn(),
                            input: crate::UserInput::new("more"),
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
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                15,
                14,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        1,
                        super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(13),
                        identity("request-2"),
                    )
                    .with_context_epoch(1),
                ),
            ),
        ],
    ));

    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("current context_epoch"));
}

// warning과 trigger가 같으면 warning 관측 구간이 사라지므로 닫힌 정책 생성 자체가
// 실패해야 하며 invalid policy가 Journal wire에 도달해서는 안 됩니다.
#[test]
fn rejects_invalid_policy_bounds_before_encoding() {
    let error = ContextPolicyChanged::try_new(
        1,
        true,
        ContextStrategy::PortableSummaryV1Alpha1,
        90,
        90,
        None,
        None,
    )
    .unwrap_err();
    assert!(error.contains("warning and trigger"));
}

// checkpoint 뒤에 요청이 하나도 없으면 교체 binding은 pre-checkpoint Anchor로 돌아가지
// 않고 source_checkpoint_sequence 하나만 사용해 같은 replay root를 이어야 합니다.
#[test]
fn replacement_can_source_a_checkpoint_only_reconstruction_exclusively() {
    let commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    });
    let commits = commits
        .into_iter()
        .map(|commit| decode(&encode(&commit).unwrap()).unwrap())
        .collect::<Vec<_>>();

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert_eq!(recovered.context_epoch(), Some(2));
    assert_eq!(
        recovered.context_checkpoint(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(recovered.model_replay().items().len(), 2);
}

// checkpoint에서 seed된 binding이 요청 없이 다시 교체되어도 newest executable source는
// 같은 checkpoint이며, 원 checkpoint epoch가 직전 binding epoch와 같을 필요는 없습니다.
#[test]
fn consecutive_idle_replacements_preserve_checkpoint_lineage() {
    let strategy = ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    };
    let mut commits = checkpoint_replacement_history(strategy);
    let transition = BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_checkpoint_sequence(JournalSequence::new(11));
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(15),
        vec![
            semantic(
                15,
                14,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    2,
                    BindingCloseReason::Replaced,
                )),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    3,
                    "managed",
                    "1.0.2",
                    identity("binding-3"),
                    identity("model"),
                    identity("session"),
                    transition,
                    ContinuationStrategy::ExactReplay {
                        executor: ReplayExecutor::LocalClient,
                        replay_profile: ReplayProfile::SemanticOnly,
                    },
                )),
            ),
        ],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(3));
    assert_eq!(recovered.context_epoch(), Some(2));
    assert_eq!(
        recovered.context_checkpoint(),
        Some(JournalSequence::new(11))
    );
    assert_eq!(recovered.model_replay().items().len(), 2);
}

// checkpoint seed에 private item이 없으면 target replay profile 변경은 손실이 아니므로
// exact transition이 새 binding의 첫 delta에서 자기 계약을 다시 선언하도록 허용합니다.
#[test]
fn replacement_allows_a_profile_change_without_private_seed_items() {
    let commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
    });

    let recovered = recover(&commits).unwrap();
    assert!(recovered.replay_contract_rebind_required());
}

// transition의 exact replay seed와 target의 이후 continuation strategy는 독립이므로
// private item이 없는 checkpoint는 backend-managed target도 안전하게 열 수 있습니다.
#[test]
fn replacement_can_seed_a_backend_managed_target_without_private_items() {
    let commits = checkpoint_replacement_history(ContinuationStrategy::BackendManagedState);

    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.binding_epoch(), Some(2));
    assert!(recovered.model_replay().items().is_empty());
    assert!(!recovered.replay_contract_rebind_required());
}

// retained private seed는 같은 binding identity와 replay profile일 때만 exact transition
// 가능하며, 다른 binding identity는 별도 lossy handoff 없이는 거부됩니다.
#[test]
fn replacement_private_seed_requires_the_same_binding_and_profile() {
    let private =
        ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", b"{}".to_vec())
            .unwrap();
    let retained_items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "done".to_owned(),
            refusal: None,
        },
        ModelReplayItem::ProviderPrivateAssistant { envelope: private },
    ];
    let mut base = current_history_with(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        retained_items.clone(),
    );
    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(7),
        JournalSequence::new(9),
        retained_items,
    )
    .unwrap();
    base.push(JournalCommit::incremental_through(
        JournalSequence::new(11),
        vec![semantic(
            12,
            11,
            JournalRecord::ContextCheckpoint(checkpoint_with(
                ModelReplayContract::new("system", Vec::new()),
                vec![retained],
                Vec::new(),
                Vec::new(),
                JournalSequence::new(10),
            )),
        )],
    ));
    let append_replacement = |commits: &mut Vec<JournalCommit>, binding_identity| {
        let transition =
            BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
                .with_source_checkpoint_sequence(JournalSequence::new(11));
        commits.push(JournalCommit::incremental_through(
            JournalSequence::new(13),
            vec![
                semantic(
                    13,
                    12,
                    JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                        1,
                        BindingCloseReason::Replaced,
                    )),
                ),
                semantic(
                    14,
                    13,
                    JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                        2,
                        "managed",
                        "1.0.1",
                        binding_identity,
                        identity("model"),
                        identity("session"),
                        transition,
                        ContinuationStrategy::ExactReplay {
                            executor: ReplayExecutor::LocalClient,
                            replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                        },
                    )),
                ),
            ],
        ));
    };

    let mut same_binding = base.clone();
    append_replacement(&mut same_binding, identity("binding"));
    let recovered = recover(&same_binding).unwrap();
    assert!(recovered.replay_contract_rebind_required());

    let mut changed_binding = base;
    append_replacement(&mut changed_binding, identity("binding-2"));
    let error = recover(&changed_binding).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("preserve every retained private")
    );
}

// checkpoint seed를 상속한 새 binding도 첫 completed request에서 자기 replay contract를
// 정확히 한 번 다시 선언해야 하며 그 선언은 old seed contract를 안전하게 교체합니다.
#[test]
fn replacement_first_delta_establishes_its_new_replay_contract() {
    let mut commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    });
    let submission_id = super::super::submission(43);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(16),
        vec![
            semantic(
                15,
                14,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn: super::super::activity().turn(),
                            input: crate::UserInput::new("after replacement"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    2,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                17,
                16,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        2,
                        super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(15),
                        identity("request-2"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    let replay = ModelReplayDelta::new(
        Some(ModelReplayContract::new("new-system", Vec::new())),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "second".to_owned(),
            refusal: None,
        }],
    );
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(20),
        vec![
            semantic(
                18,
                17,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn: super::super::activity().turn(),
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                19,
                18,
                JournalRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        2,
                        super::super::activity().turn_id(),
                        JournalSequence::new(16),
                        replay,
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                20,
                19,
                JournalRecord::BackendResumableOutcome(
                    BackendResumableOutcome::new(
                        2,
                        super::super::activity().turn_id(),
                        JournalSequence::new(16),
                        Some(identity("outcome-2")),
                        Some(JournalSequence::new(18)),
                    )
                    .with_context_epoch(2),
                ),
            ),
            semantic(
                21,
                20,
                JournalRecord::ContinuationAnchor(
                    ContinuationAnchor::new(
                        2,
                        JournalSequence::new(16),
                        JournalSequence::new(19),
                        JournalSequence::new(19),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));

    let recovered = recover(&commits).unwrap();
    assert_eq!(
        recovered.model_replay().contract(),
        Some(&ModelReplayContract::new("new-system", Vec::new()))
    );
    assert_eq!(recovered.model_replay().items().len(), 3);

    let retained = ContextRetainedGroup::try_new(
        JournalSequence::new(17),
        JournalSequence::new(19),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "second".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    let inherited_seed =
        ContextLoss::visible_prefix_summarized(JournalSequence::new(13), JournalSequence::new(13))
            .unwrap();
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(21),
        vec![semantic(
            22,
            21,
            JournalRecord::ContextCheckpoint(checkpoint_for(
                CheckpointLineage {
                    epoch: 2,
                    previous_context_epoch: 2,
                    successor_context_epoch: 3,
                    source_anchor_sequence: JournalSequence::new(20),
                    source_journal_boundary: JournalSequence::new(20),
                },
                ModelReplayContract::new("new-system", Vec::new()),
                vec![retained],
                Vec::new(),
                vec![inherited_seed],
            )),
        )],
    ));
    let recovered = recover(&commits).unwrap();
    assert_eq!(recovered.context_epoch(), Some(3));
    assert_eq!(recovered.model_replay().items().len(), 2);
}

// replacement checkpoint 뒤 accepted request가 durable하지만 matching Anchor가 없으면
// transition source로 되돌아가 재전송하지 않고 executable continuation을 거부합니다.
#[test]
fn replacement_with_an_unanchored_request_does_not_fall_back_to_checkpoint() {
    let mut commits = checkpoint_replacement_history(ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    });
    let submission_id = super::super::submission(44);
    let operation_id = OperationId::from(submission_id);
    commits.push(JournalCommit::incremental_through(
        JournalSequence::new(16),
        vec![
            semantic(
                15,
                14,
                JournalRecord::CommandCommitted(
                    crate::journal::CommittedCommand::submission(
                        AgentCommand::SteerTurn {
                            turn: super::super::activity().turn(),
                            input: crate::UserInput::new("uncertain"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                16,
                15,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    2,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "managed.request/v1",
                    None,
                    None,
                    DetailAvailability::Unpersisted,
                )),
            ),
            semantic(
                17,
                16,
                JournalRecord::BackendRequestAccepted(
                    BackendRequestAccepted::new(
                        2,
                        super::super::activity().turn_id(),
                        operation_id,
                        JournalSequence::new(15),
                        identity("request-uncertain"),
                    )
                    .with_context_epoch(2),
                ),
            ),
        ],
    ));
    let recovered = recover(&commits).unwrap();

    let error = crate::session_repository::build_continuation(
        recovered,
        super::super::activity().session_id(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("no newest durable"));
}
