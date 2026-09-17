#[cfg(test)]
use std::env;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::iter;
#[cfg(test)]
use std::path::PathBuf;
use std::{slice, time::Duration};

use super::{
    AgentCommand, AgentEvent, JournalCommit, JournalRecord, MessageEnded, MessageOutcome,
    MessageSegment, MessageSegmenter, MessageStream, MessageTerminal, activity,
    descriptor_with_path, encode, recover, sequenced, submission,
};
#[cfg(test)]
use crate::journal::CommittedCommand;
#[cfg(test)]
use crate::session_repository;
#[cfg(test)]
use crate::session_repository::RepositorySequence;
#[cfg(test)]
use crate::session_repository::SessionUsageProjection;
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, BackendBindingEvidence,
    BackendIdentity, BackendResumeSource, ContinuationStrategy, JournalSequence,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile, TurnId, TurnOutcome, TurnRef,
    fixture_descriptor, fixture_session,
    journal::codec::{
        BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
        BackendRequestAccepted, BackendResumableOutcome, BindingCloseReason, BindingTransition,
        CacheState, ContextCheckpoint, ContextLoss, ContextPolicyChanged, ContextRetainedGroup,
        ContextStrategy, ContextSummaryUsage, ContinuationAnchor, DetailAvailability,
        ExchangeDirection, ExchangeKind, ForkExactReplay, ForkGroup, ForkHistoryCoordinate,
        ForkHistoryEntry, ForkItemCoordinate, ForkItemOrigin, ForkSeed, ForkSource,
        ForkSourcePoint, InitialForkSeed, ModelReplayDeltaRecord, OperationId, ReplaySequence,
        SequencedJournalRecord, TransitionMode, VersionedIdentity, decode,
    },
    provider_private_schema,
    session_repository::{
        LocalSessionReader, LocalSessionRepository, StoredSessionReader,
        journal::JournalRepository, read_stored_session, read_stored_session_continuation,
    },
};

mod forks;
mod limits;
mod messages;
mod records;

fn fork_binding(profile: ReplayProfile) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "managed",
        "1.0.0",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "source"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: profile,
        },
    )
}

fn fork_records(private: bool) -> Vec<JournalRecord> {
    let parent = fixture_session(81);
    let child = fixture_session(82);
    let profile = if private {
        ReplayProfile::ProviderPrivateLocalPlaintext
    } else {
        ReplayProfile::SemanticOnly
    };
    let source = fork_binding(profile);
    let mut items = vec![ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: "exact\r\nvisible".into(),
        refusal: None,
    }];
    if private {
        items.push(ModelReplayItem::ProviderPrivateAssistant {
            envelope: ProviderPrivateReplayEnvelope::new(
                provider_private_schema(profile).unwrap(),
                br#"{"reasoning_content":"private\r\nbytes","content":"exact\r\nvisible"}"#
                    .to_vec(),
            )
            .unwrap(),
        });
    }
    let origins = (0..items.len())
        .map(|index| {
            let coordinate =
                ForkItemCoordinate::new(parent, 3, 2, JournalSequence::new(7), index).unwrap();
            ForkItemOrigin::new(coordinate, coordinate, source.clone()).unwrap()
        })
        .collect();
    let groups = vec![ForkGroup::new(0, items.len()).unwrap()];
    let exact = ForkExactReplay::new(
        ModelReplayContract::new("exact system contract", vec![]),
        items,
        origins,
        groups,
    )
    .unwrap();
    let seed = InitialForkSeed::new(
        child,
        parent,
        ForkSource::Anchor(
            ForkSourcePoint::new(
                3,
                2,
                JournalSequence::new(9),
                JournalSequence::new(8),
                source.clone(),
            )
            .unwrap(),
        ),
        ForkSeed::ExactReplay(exact),
        vec![],
        2,
    )
    .unwrap();
    vec![
        JournalRecord::SessionDescriptor(fixture_descriptor(child)),
        JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child }),
        JournalRecord::InitialForkSeed(Box::new(seed)),
        JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
            1,
            source.backend_kind(),
            source.backend_version(),
            VersionedIdentity::new(
                source.binding_identity().schema(),
                source.binding_identity().value(),
            ),
            VersionedIdentity::new(
                source.model_identity().schema(),
                source.model_identity().value(),
            ),
            VersionedIdentity::new("locator/v1", "child"),
            BindingTransition::initial_fork(JournalSequence::new(2)),
            source.continuation_strategy(),
        )),
        JournalRecord::ContextPolicyChanged(
            ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65_536),
            )
            .unwrap(),
        ),
    ]
}

fn fork_commit(records: Vec<JournalRecord>) -> JournalCommit {
    let cutoff = JournalSequence::new((records.len() - 1) as u64);
    let entries = records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            let sequence = ReplaySequence::new(index as u64 + 1);
            if index == 0 {
                SequencedJournalRecord::storage(sequence, record)
            } else {
                SequencedJournalRecord::with_journal_sequence(
                    sequence,
                    JournalSequence::new(index as u64),
                    record,
                )
            }
        })
        .collect();
    JournalCommit::incremental_through(cutoff, entries)
}

fn fork_child_commit(first: u64, records: Vec<JournalRecord>) -> JournalCommit {
    let cutoff = first + records.len() as u64 - 1;
    JournalCommit::incremental_through(
        JournalSequence::new(cutoff),
        records
            .into_iter()
            .enumerate()
            .map(|(index, record)| {
                let sequence = first + index as u64;
                SequencedJournalRecord::with_journal_sequence(
                    ReplaySequence::new(sequence + 1),
                    JournalSequence::new(sequence),
                    record,
                )
            })
            .collect(),
    )
}

fn fork_child_request() -> (TurnRef, JournalCommit) {
    let turn = TurnRef::new(fixture_session(82), TurnId::new(1.try_into().unwrap()));
    let submission_id = submission(83);
    let operation = OperationId::from(submission_id);
    let request = fork_child_commit(
        5,
        vec![
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new("child followup"),
                    },
                    submission_id,
                )
                .unwrap(),
            ),
            JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                1,
                operation,
                ExchangeKind::Request,
                ExchangeDirection::YoToBackend,
                "managed.request/v1",
                None,
                None,
                DetailAvailability::Unpersisted,
            )),
            JournalRecord::BackendRequestAccepted(
                BackendRequestAccepted::new(
                    1,
                    turn.turn_id(),
                    operation,
                    JournalSequence::new(6),
                    VersionedIdentity::new("request/v1", "child-request"),
                )
                .with_context_epoch(1),
            ),
        ],
    );
    (turn, request)
}

struct ForkRepositoryDirectory(PathBuf);

impl Drop for ForkRepositoryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fork_private_answer(text: &str) -> Vec<ModelReplayItem> {
    vec![ModelReplayItem::Message { role: ModelReplayRole::Assistant, content: text.into(), refusal: None },
        ModelReplayItem::ProviderPrivateAssistant { envelope: ProviderPrivateReplayEnvelope::new(
            provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
            serde_json::to_vec(&serde_json::json!({"reasoning_content":format!("private {text}"),"content":text})).unwrap()).unwrap() }]
}

fn fork_two_group_bootstrap() -> (JournalCommit, ForkExactReplay) {
    let mut records = fork_records(true);
    let JournalRecord::InitialForkSeed(seed) = &records[2] else {
        panic!("seed")
    };
    let ForkSeed::ExactReplay(first) = seed.seed() else {
        panic!("exact")
    };
    let mut items = first.items().to_vec();
    items.extend(fork_private_answer("keep imported group one"));
    let origins = (0..items.len())
        .map(|index| {
            let coordinate =
                ForkItemCoordinate::new(fixture_session(81), 3, 2, JournalSequence::new(7), index)
                    .unwrap();
            ForkItemOrigin::new(
                coordinate,
                coordinate,
                fork_binding(ReplayProfile::ProviderPrivateLocalPlaintext),
            )
            .unwrap()
        })
        .collect();
    let exact = ForkExactReplay::new(
        first.contract().clone(),
        items,
        origins,
        vec![ForkGroup::new(0, 2).unwrap(), ForkGroup::new(2, 4).unwrap()],
    )
    .unwrap();
    records[2] = JournalRecord::InitialForkSeed(Box::new(
        InitialForkSeed::new(
            fixture_session(82),
            fixture_session(81),
            seed.source().clone(),
            ForkSeed::ExactReplay(exact.clone()),
            vec![],
            2,
        )
        .unwrap(),
    ));
    (fork_commit(records), exact)
}

fn fork_private_turn(
    first: u64,
    turn_id: u64,
    context_epoch: u64,
) -> (JournalCommit, JournalCommit, Vec<ModelReplayItem>) {
    fork_private_turn_at_epoch(1, first, turn_id, context_epoch)
}

fn fork_private_turn_at_epoch(
    epoch: u64,
    first: u64,
    turn_id: u64,
    context_epoch: u64,
) -> (JournalCommit, JournalCommit, Vec<ModelReplayItem>) {
    let turn = TurnRef::new(
        fixture_session(82),
        TurnId::new(turn_id.try_into().unwrap()),
    );
    let submission_id = submission(u8::try_from(90 + turn_id).unwrap());
    let operation = OperationId::from(submission_id);
    let input = format!("child request {turn_id}");
    let request = fork_child_commit(
        first,
        vec![
            JournalRecord::CommandCommitted(
                CommittedCommand::submission(
                    AgentCommand::StartTurn {
                        turn,
                        input: crate::UserInput::new(input.clone()),
                    },
                    submission_id,
                )
                .unwrap(),
            ),
            JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                epoch,
                operation,
                ExchangeKind::Request,
                ExchangeDirection::YoToBackend,
                "managed.request/v1",
                None,
                None,
                DetailAvailability::Unpersisted,
            )),
            JournalRecord::BackendRequestAccepted(
                BackendRequestAccepted::new(
                    epoch,
                    turn.turn_id(),
                    operation,
                    JournalSequence::new(first + 1),
                    VersionedIdentity::new("request/v1", format!("request-{turn_id}")),
                )
                .with_context_epoch(context_epoch),
            ),
        ],
    );
    let mut suffix = vec![ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: input,
        refusal: None,
    }];
    suffix.extend(fork_private_answer(&format!("child answer {turn_id}")));
    let complete = fork_child_commit(
        first + 3,
        vec![
            JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                turn,
                outcome: TurnOutcome::Completed,
            }),
            JournalRecord::ModelReplayDelta(
                ModelReplayDeltaRecord::new(
                    epoch,
                    turn.turn_id(),
                    JournalSequence::new(first + 2),
                    ModelReplayDelta::new(
                        (epoch > 1)
                            .then(|| ModelReplayContract::new("exact system contract", vec![])),
                        suffix.clone(),
                    ),
                )
                .with_context_epoch(context_epoch),
            ),
            JournalRecord::BackendResumableOutcome(
                BackendResumableOutcome::new(
                    epoch,
                    turn.turn_id(),
                    JournalSequence::new(first + 2),
                    Some(VersionedIdentity::new(
                        "outcome/v1",
                        format!("outcome-{turn_id}"),
                    )),
                    Some(JournalSequence::new(first + 4)),
                )
                .with_context_epoch(context_epoch),
            ),
            JournalRecord::ContinuationAnchor(
                ContinuationAnchor::new(
                    epoch,
                    JournalSequence::new(first + 2),
                    JournalSequence::new(first + 5),
                    JournalSequence::new(first + 5),
                )
                .with_context_epoch(context_epoch),
            ),
        ],
    );
    (request, complete, suffix)
}

fn fork_portable_summary() -> &'static str {
    "# Context Checkpoint\n## Current Objective\nContinue.\n## Active Constraints\nNone.\n## Decisions\nNone.\n## Verified Progress\nEarlier group summarized.\n## Current State\nIdle.\n## Unknown or Unverified\nNone.\n## Next Actions\nContinue.\n## Critical References\nNone."
}

fn fork_checkpoint(
    previous: u64,
    anchor: u64,
    groups: Vec<ContextRetainedGroup>,
    losses: Vec<ContextLoss>,
    contract: &ModelReplayContract,
) -> ContextCheckpoint {
    fork_checkpoint_at_epoch(1, previous, anchor, groups, losses, contract)
}

fn fork_checkpoint_at_epoch(
    epoch: u64,
    previous: u64,
    anchor: u64,
    groups: Vec<ContextRetainedGroup>,
    losses: Vec<ContextLoss>,
    contract: &ModelReplayContract,
) -> ContextCheckpoint {
    let usage = ContextSummaryUsage::try_new(serde_json::json!({"schema":"yo.model-usage-receipt/v1","response_id":"summary","round":1,
        "provider":"test","account":"default","model":"test","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://example.invalid/",
        "usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"reasoning_tokens":0},"cache_read_input_tokens":{"availability":"unsupported"}})).unwrap();
    let first = groups.first().map(ContextRetainedGroup::first_sequence);
    ContextCheckpoint::try_new(
        epoch,
        previous,
        previous + 1,
        JournalSequence::new(anchor),
        JournalSequence::new(anchor),
        1,
        ContextStrategy::PortableSummaryV1Alpha1,
        100_000,
        90_000,
        20_000,
        contract.clone(),
        fork_portable_summary(),
        groups,
        first,
        vec![],
        losses,
        usage,
    )
    .unwrap()
}

fn fork_import_retained(exact: &ForkExactReplay, index: usize) -> ContextRetainedGroup {
    let group = exact.groups()[index];
    ContextRetainedGroup::try_imported(
        JournalSequence::new(2),
        index,
        exact.items()[group.first_item()..group.end_item()].to_vec(),
        vec![3],
    )
    .unwrap()
}

fn fork_first_losses(exact: &ForkExactReplay) -> Vec<ContextLoss> {
    let ModelReplayItem::ProviderPrivateAssistant { envelope } = &exact.items()[1] else {
        panic!("private")
    };
    vec![
        ContextLoss::visible_prefix_summarized(JournalSequence::new(2), JournalSequence::new(2))
            .unwrap(),
        ContextLoss::provider_private_dropped(
            envelope.schema(),
            envelope.payload().len() as u64,
            JournalSequence::new(2),
        )
        .unwrap(),
    ]
}

fn fork_exact_transition() -> BindingTransition {
    BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
        .with_source_initial_fork_sequence(JournalSequence::new(2))
}

fn fork_replacement(
    first: u64,
    previous: u64,
    transition: BindingTransition,
    model: &str,
    profile: ReplayProfile,
) -> JournalCommit {
    let strategy = fork_binding(profile).continuation_strategy();
    fork_child_commit(
        first,
        vec![
            JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                previous,
                BindingCloseReason::Replaced,
            )),
            JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                previous + 1,
                "managed",
                "1.0.0",
                VersionedIdentity::new("binding/v1", "account"),
                VersionedIdentity::new("model/v1", model),
                VersionedIdentity::new("locator/v1", format!("child-epoch-{}", previous + 1)),
                transition,
                strategy,
            )),
        ],
    )
}

fn fork_captured_bootstrap(child: crate::SessionId, seed: InitialForkSeed) -> JournalCommit {
    let mut records = fork_records(true);
    records[0] = JournalRecord::SessionDescriptor(fixture_descriptor(child));
    records[1] = JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child });
    records[2] = JournalRecord::InitialForkSeed(Box::new(seed));
    fork_commit(records)
}
