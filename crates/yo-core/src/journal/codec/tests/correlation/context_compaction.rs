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

mod artifacts;
mod checkpoint;
mod replacement;
mod requests;
