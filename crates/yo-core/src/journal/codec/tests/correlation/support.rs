use crate::{
    AgentCommand, AgentEvent, ContinuationStrategy, JournalSequence, ModelReplayContract,
    ModelReplayDelta, ModelReplayItem, ModelReplayRole, ReplayExecutor, TurnOutcome,
    journal::codec::{
        BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
        BackendResumableOutcome, BindingTransition, CacheState, ContinuationAnchor,
        DetailAvailability, ExchangeDirection, ExchangeKind, JournalCommit, JournalRecord,
        ModelReplayDeltaRecord, OperationId, ReplaySequence, SequencedJournalRecord,
        TransitionMode, VersionedIdentity,
    },
};

pub(super) fn model_replay_delta() -> ModelReplayDelta {
    ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
    )
}

pub(super) fn semantic(
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

pub(super) fn identity(name: &str) -> VersionedIdentity {
    VersionedIdentity::new(format!("yo.test.{name}/v1"), format!("{name}:value"))
}

fn binding_opened(replay_profile: crate::ReplayProfile) -> JournalRecord {
    JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
        1,
        "codex",
        "1.0.0",
        identity("binding"),
        identity("model"),
        identity("session"),
        BindingTransition::new(TransitionMode::Initial, CacheState::NotApplicable, None),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile,
        },
    ))
}

pub(super) fn valid_history() -> Vec<JournalCommit> {
    valid_history_with_replay(model_replay_delta())
}

pub(super) fn valid_history_with_replay(replay_delta: ModelReplayDelta) -> Vec<JournalCommit> {
    valid_history_with_profile_and_replay(crate::ReplayProfile::SemanticOnly, replay_delta)
}

pub(super) fn valid_history_with_profile_and_replay(
    replay_profile: crate::ReplayProfile,
    replay_delta: ModelReplayDelta,
) -> Vec<JournalCommit> {
    let descriptor =
        JournalCommit::descriptor(super::super::descriptor_with_path(b"/workspace".to_vec()));
    let opened = JournalCommit::incremental_through(
        JournalSequence::new(2),
        vec![
            semantic(
                2,
                1,
                JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                    session_id: super::super::activity().session_id(),
                }),
            ),
            semantic(3, 2, binding_opened(replay_profile)),
        ],
    );
    let submission_id = super::super::submission(11);
    let operation_id = OperationId::from(submission_id);
    let request = JournalCommit::incremental_through(
        JournalSequence::new(5),
        vec![
            semantic(
                4,
                3,
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
                5,
                4,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
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
                6,
                5,
                JournalRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                    1,
                    super::super::activity().turn_id(),
                    operation_id,
                    JournalSequence::new(4),
                    identity("request"),
                )),
            ),
        ],
    );
    let completed = JournalCommit::incremental_through(
        JournalSequence::new(9),
        vec![
            semantic(
                7,
                6,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn: super::super::activity().turn(),
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                8,
                7,
                JournalRecord::ModelReplayDelta(ModelReplayDeltaRecord::new(
                    1,
                    super::super::activity().turn_id(),
                    JournalSequence::new(5),
                    replay_delta,
                )),
            ),
            semantic(
                9,
                8,
                JournalRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                    1,
                    super::super::activity().turn_id(),
                    JournalSequence::new(5),
                    Some(identity("outcome")),
                    Some(JournalSequence::new(7)),
                )),
            ),
            semantic(
                10,
                9,
                JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
                    1,
                    JournalSequence::new(5),
                    JournalSequence::new(8),
                    JournalSequence::new(8),
                )),
            ),
        ],
    );
    vec![descriptor, opened, request, completed]
}
