use std::sync::{Arc, Mutex};

use super::super::*;
use crate::{
    BackendCapabilities, BackendIdentity, BackendOutcomeEvidence, BackendRequestEvidence,
    BackendScriptStep, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile, ScriptedBackend, TurnId,
    UserInput,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, RepositoryEntry, RepositoryError,
        RepositorySequence, SessionRepository, SessionWriterRepository,
    },
};

#[derive(Clone, Default)]
struct MemoryRepository(Arc<Mutex<Vec<RepositoryEntry>>>);

impl SessionRepository for MemoryRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        let mut entries = self.0.lock().unwrap();
        let sequence = RepositorySequence::new(u64::try_from(entries.len()).unwrap() + 1);
        entries.push(RepositoryEntry::new(sequence, record));
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
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

impl SessionWriterRepository for MemoryRepository {
    fn acquire_session_writer(&mut self, _session_id: SessionId) -> Result<(), RepositoryError> {
        Ok(())
    }
}

fn binding(identity: &str, strategy: ContinuationStrategy) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "managed",
        "1.0.0",
        BackendIdentity::new("test.binding/v1", identity),
        BackendIdentity::new("test.model/v1", "model"),
        BackendIdentity::new("test.session/v1", "session"),
        strategy,
    )
}

fn native_binding(identity: &str, model: &str, locator: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "codex-app-server",
        "codex_cli_rs/0.152.1",
        BackendIdentity::new("codex.app-server/thread-binding/v2", identity),
        BackendIdentity::new("codex.app-server/model-and-provider/v1", model),
        BackendIdentity::new("codex.app-server/thread-locator/v1", locator),
        ContinuationStrategy::BackendManagedState,
    )
}

fn durable_runtime(
    backend: ScriptedBackend,
    session_id: SessionId,
) -> (AgentRuntime<Box<dyn AgentBackend + Send>>, MemoryRepository) {
    let repository = MemoryRepository::default();
    let journal = SessionJournal::with_repository_and_descriptor(
        Box::new(repository.clone()),
        crate::fixture_descriptor(session_id),
    );
    let mut runtime = AgentRuntime::with_journal(
        Box::new(backend.with_capabilities(BackendCapabilities::none().with_native_model_rebind()))
            as Box<dyn AgentBackend + Send>,
        journal,
    );
    runtime.initialize_durability();
    (runtime, repository)
}

// retained private replay는 같은 binding identity와 replay profile에서만 손실 없이
// 교체할 수 있고, private item이 없는 seed는 다른 target 전략을 허용합니다.
#[test]
fn replacement_binding_compatibility_is_conditional_on_private_replay() {
    let private_strategy = ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
    };
    let previous = binding("same", private_strategy);
    let mut private_replay = ModelReplay::default();
    private_replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            vec![
                ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: "answer".to_owned(),
                    refusal: None,
                },
                ModelReplayItem::ProviderPrivateAssistant {
                    envelope: ProviderPrivateReplayEnvelope::new(
                        "kimi.assistant-message/v1alpha1",
                        b"{}".to_vec(),
                    )
                    .unwrap(),
                },
            ],
        ))
        .unwrap();

    assert!(valid_replacement_binding(
        &previous,
        &binding("same", private_strategy),
        &private_replay,
    ));
    assert!(!valid_replacement_binding(
        &previous,
        &binding("different", private_strategy),
        &private_replay,
    ));

    let semantic_replay = ModelReplay::from_checkpoint(
        ModelReplayContract::new("system", Vec::new()),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "summary".to_owned(),
            refusal: None,
        }],
    )
    .unwrap();
    assert!(valid_replacement_binding(
        &previous,
        &binding("different", ContinuationStrategy::BackendManagedState),
        &semantic_replay,
    ));
}

// 아직 request를 수락하지 않은 backend-managed binding은 Anchor 없이 provider-native
// fork할 수 있고, distinct binding/model/locator를 한 atomic transition으로 엽니다.
#[test]
fn native_model_rebind_allows_a_source_free_unused_binding() {
    let session_id = crate::fixture_session(41);
    let source = native_binding("binding-a", "model-a", "thread-a");
    let replacement = native_binding("binding-b", "model-b", "thread-b");
    let current = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(source.clone()),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let target = BackendResumeTarget::for_model_rebind(session_id, 1, source, None);
    let candidate = ScriptedBackend::new([
        BackendScriptStep::RebindModel {
            target: Box::new(target),
            evidence: replacement.clone(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_native_model_rebind());
    let (mut runtime, _) = durable_runtime(current, session_id);

    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    if let Err(error) = runtime.replace_backend(Box::new(candidate)) {
        panic!("native model rebind failed: {}", error.primary);
    }

    assert_eq!(runtime.binding.as_ref(), Some(&replacement));
    let entries = runtime.journal.semantic_entries();
    let opened = entries
        .iter()
        .rev()
        .find_map(|entry| match entry.record() {
            crate::journal::SemanticRecord::BackendBindingOpened(binding) => Some(binding),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        opened.transition().mode(),
        crate::journal::codec::TransitionMode::BackendNativeModelRebind
    );
    assert_eq!(
        opened.transition().cache(),
        crate::journal::codec::CacheState::Unknown
    );
    assert!(opened.transition().source_anchor_sequence().is_none());
    runtime.shutdown().unwrap();
}

// source binding이 request를 수락했다면 runtime은 그 Turn의 newest Anchor를 fork
// target과 durable transition 모두에 결속하고 임의의 source-free 전환을 허용하지 않습니다.
#[test]
fn native_model_rebind_uses_the_newest_source_anchor_after_model_work() {
    let session_id = crate::fixture_session(42);
    let turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(1).unwrap()),
    );
    let source = native_binding("binding-a", "model-a", "thread-a");
    let replacement = native_binding("binding-b", "model-b", "thread-b");
    let request = BackendRequestEvidence::new(
        "codex.app-server/turn-start/v1",
        BackendIdentity::new("codex.app-server/json-rpc-request/v1", "2"),
        BackendIdentity::new("codex.app-server/accepted-request/v1", "turn-a"),
    );
    let current = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(source.clone()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: UserInput::new("hello"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "codex.app-server/turn-outcome/v1",
                "turn-a",
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let (mut runtime, mut repository) = durable_runtime(current, session_id);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn,
                input: UserInput::new("hello"),
            },
            SubmissionId::new().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished { .. })
    ));
    let Some(BackendResumeSource::ContinuationAnchor(anchor)) = runtime.resume_source else {
        panic!("completed backend-managed work has a continuation Anchor");
    };
    let target = BackendResumeTarget::for_model_rebind(session_id, 1, source, Some(anchor));
    let candidate = ScriptedBackend::new([
        BackendScriptStep::RebindModel {
            target: Box::new(target),
            evidence: replacement,
        },
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_native_model_rebind());

    if let Err(error) = runtime.replace_backend(Box::new(candidate)) {
        panic!("native model rebind failed: {}", error.primary);
    }

    let entries = runtime.journal.semantic_entries();
    let opened = entries
        .iter()
        .rev()
        .find_map(|entry| match entry.record() {
            crate::journal::SemanticRecord::BackendBindingOpened(binding) => Some(binding),
            _ => None,
        })
        .unwrap();
    assert_eq!(opened.transition().source_anchor_sequence(), Some(anchor));
    let continuation =
        crate::session_repository::recover_stored_session_continuation(&mut repository, session_id)
            .unwrap();
    assert!(!continuation.target().binding_has_accepted_request());
    runtime.shutdown().unwrap();
}

// request는 수락됐지만 resumable outcome이 없어 Anchor를 만들 수 없었던 binding을
// unused binding처럼 취급하지 않고 candidate RPC 전에 닫습니다.
#[test]
fn native_model_rebind_rejects_an_unanchored_accepted_request() {
    let session_id = crate::fixture_session(43);
    let turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(1).unwrap()),
    );
    let source = native_binding("binding-a", "model-a", "thread-a");
    let current = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(source),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: UserInput::new("fail"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "codex.app-server/turn-start/v1",
                BackendIdentity::new("codex.app-server/json-rpc-request/v1", "2"),
                BackendIdentity::new("codex.app-server/accepted-request/v1", "turn-a"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(Failure::new("failed")),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let (mut runtime, _) = durable_runtime(current, session_id);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn,
                input: UserInput::new("fail"),
            },
            SubmissionId::new().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished { .. })
    ));
    let candidate = ScriptedBackend::new([BackendScriptStep::Shutdown(Ok(()))])
        .with_capabilities(BackendCapabilities::none().with_native_model_rebind());

    let error = runtime.replace_backend(Box::new(candidate)).unwrap_err();

    assert!(
        error
            .primary
            .to_string()
            .contains("newest durable continuation Anchor")
    );
    runtime.shutdown().unwrap();
}

// 새 request가 수락되는 순간 이전 Turn의 Anchor는 더 이상 최신 source가 아닙니다.
// 후속 Turn 실패 뒤에는 candidate fork 자체를 호출하기 전에 rebind를 닫아야 합니다.
#[test]
fn native_model_rebind_rejects_a_stale_anchor_before_candidate_fork() {
    let session_id = crate::fixture_session(44);
    let first_turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(1).unwrap()),
    );
    let second_turn = TurnRef::new(
        session_id,
        TurnId::new(std::num::NonZeroU64::new(2).unwrap()),
    );
    let source = native_binding("binding-a", "model-a", "thread-a");
    let request = |id: &str| {
        BackendRequestEvidence::new(
            "codex.app-server/turn-start/v1",
            BackendIdentity::new("codex.app-server/json-rpc-request/v1", id),
            BackendIdentity::new("codex.app-server/accepted-request/v1", id),
        )
    };
    let current = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(source.clone()),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: first_turn,
                input: UserInput::new("first"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request("request-1")),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: first_turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "codex.app-server/turn-outcome/v1",
                "request-1",
            )),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: second_turn,
                input: UserInput::new("second"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request("request-2")),
        },
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: second_turn,
            outcome: TurnOutcome::Failed(Failure::new("failed")),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let (mut runtime, _) = durable_runtime(current, session_id);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: first_turn,
                input: UserInput::new("first"),
            },
            SubmissionId::new().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished { .. })
    ));
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: second_turn,
                input: UserInput::new("second"),
            },
            SubmissionId::new().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished { .. })
    ));
    let candidate = ScriptedBackend::new([BackendScriptStep::Shutdown(Ok(()))])
        .with_capabilities(BackendCapabilities::none().with_native_model_rebind());
    let previous_epoch = runtime.binding_epoch;
    let previous_binding = runtime.binding.clone();
    let previous_records = runtime.journal.semantic_entries().len();

    let error = runtime.replace_backend(Box::new(candidate)).unwrap_err();

    assert!(
        error
            .primary
            .to_string()
            .contains("newest durable continuation Anchor")
    );
    assert_eq!(runtime.binding_epoch, previous_epoch);
    assert_eq!(runtime.binding, previous_binding);
    assert_eq!(runtime.journal.semantic_entries().len(), previous_records);
    runtime.shutdown().unwrap();
}
