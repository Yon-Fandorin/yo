#[cfg(test)]
use std::num::NonZeroU64;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::*;
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, AgentIntent, AgentRuntime, AgentSession, AgentSessionPoll, BackendBindingEvidence,
    BackendCapabilities, BackendCommandEvidence, BackendEvent, BackendIdentity,
    BackendOutcomeEvidence, BackendRequestEvidence, BackendResumeTarget, BackendScriptStep,
    CommandAdmission, ContextPolicyChanged, ContextStrategy, ContinuationStrategy,
    ImageInputCapability, InputAdmissionHost, InputImage, InputImageHistory, InputImageSnapshot,
    InputSubmission, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ReplayExecutor, RuntimePoll, ScriptedBackend, SessionId, SubmissionId, SubmissionOutcome,
    SubmissionRejection, SubmissionRejectionKind, TurnId, TurnRef, UserInput,
    journal::{SessionJournal, codec::JournalRecord},
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, DurableRecordKind, RepositoryEntry,
        RepositoryError, RepositorySequence, SessionRepository, SessionWriterRepository,
    },
};

mod catalog;
mod fork;
mod limits;
mod recovery;

#[derive(Clone, Debug, Default)]
struct MemoryRepository {
    entries: Arc<Mutex<Vec<RepositoryEntry>>>,
    fail_append: Arc<AtomicBool>,
}

impl SessionRepository for MemoryRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        if self.fail_append.swap(false, Ordering::AcqRel) {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "injected append failure".to_owned(),
            }));
        }
        let mut entries = self.entries.lock().unwrap();
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
            .entries
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

fn binding() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "codex-app-server",
        "codex_cli_rs/0.146.0",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            r#"{"sessionId":"thread-a","threadId":"thread-a"}"#,
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"gpt-test","provider":"openai"}"#,
        ),
        BackendIdentity::new("codex.app-server/thread-locator/v1", "thread-a"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: crate::ReplayProfile::SemanticOnly,
        },
    )
}

fn managed_binding() -> BackendBindingEvidence {
    let source = binding();
    BackendBindingEvidence::new(
        source.backend_kind(),
        source.backend_version(),
        source.binding_identity().clone(),
        source.model_identity().clone(),
        source.session_locator().clone(),
        ContinuationStrategy::BackendManagedState,
    )
}

fn replacement_binding() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        binding().backend_kind(),
        "replacement/v1",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            r#"{"sessionId":"thread-b","threadId":"thread-b"}"#,
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"replacement-model","provider":"openai"}"#,
        ),
        binding().session_locator().clone(),
        binding().continuation_strategy(),
    )
}

fn second_replacement_binding() -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        binding().backend_kind(),
        "replacement/v2",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            r#"{"sessionId":"thread-c","threadId":"thread-c"}"#,
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            r#"{"model":"second-replacement-model","provider":"openai"}"#,
        ),
        binding().session_locator().clone(),
        binding().continuation_strategy(),
    )
}

fn stored_submission() -> SubmissionId {
    SubmissionId::from_uuid(uuid::Builder::from_random_bytes([9; 16]).into_uuid()).unwrap()
}

fn wait_for_submission_outcome(session: &mut AgentSession) -> SubmissionOutcome {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(outcome) = session.take_submission_outcome() {
            return outcome;
        }
        assert!(Instant::now() < deadline, "submission outcome timed out");
        session
            .poll()
            .expect("submission rejection must keep the resumed worker healthy");
        thread::sleep(Duration::from_millis(1));
    }
}

fn enqueue_submission(session: &mut AgentSession, mut admission: CommandAdmission) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match admission {
            CommandAdmission::Queued => return,
            CommandAdmission::Backpressured(pending) => {
                assert!(
                    Instant::now() < deadline,
                    "submission remained backpressured past the test deadline"
                );
                thread::sleep(Duration::from_millis(1));
                admission = session
                    .retry(pending)
                    .expect("the submitted command must remain retryable");
            },
            CommandAdmission::Rejected { .. } => {
                panic!("the test submission was rejected instead of queued")
            },
        }
    }
}

struct TestInputAdmission;

impl InputAdmissionHost for TestInputAdmission {
    fn validate(&self, _: &UserInput) -> Result<(), SubmissionRejection> {
        Ok(())
    }

    fn validate_images(&self, _: &UserInput) -> Result<(), SubmissionRejection> {
        Ok(())
    }
}

fn image_input() -> UserInput {
    let snapshot: InputImageSnapshot = serde_json::from_str(
        r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#,
    )
    .unwrap();
    UserInput::new("[image]")
        .with_images(vec![InputImage::new(0..7, 70, snapshot).unwrap()])
        .unwrap()
}

fn durable_resumable_session() -> (MemoryRepository, StoredSessionContinuation) {
    durable_resumable_session_with_binding(binding(), UserInput::new("resume me"))
}

fn durable_resumable_image_session() -> (MemoryRepository, StoredSessionContinuation) {
    durable_resumable_session_with_binding(managed_binding(), image_input())
}

fn durable_resumable_session_with_binding(
    binding: BackendBindingEvidence,
    input: UserInput,
) -> (MemoryRepository, StoredSessionContinuation) {
    let session_id = crate::fixture_session(1);
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let submission = stored_submission();
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let request = BackendRequestEvidence::new(
        "codex.app-server/turn-start/v1",
        BackendIdentity::new("codex.app-server/json-rpc-request/v1", "3"),
        BackendIdentity::new(
            "codex.app-server/accepted-request/v1",
            r#"{"jsonRpcId":3,"turnId":"turn-a"}"#,
        ),
    );
    let image_capability = if input.images().is_empty() {
        ImageInputCapability::Unknown
    } else {
        ImageInputCapability::Supported {
            maximum_occurrences: 16,
            maximum_image_bytes: InputImageSnapshot::MAX_BYTES as u64,
            maximum_input_bytes: InputImageSnapshot::MAX_BYTES as u64,
        }
    };
    let mut steps = vec![BackendScriptStep::AcceptCommandWithEvidence {
        command: AgentCommand::CreateSession { session_id },
        evidence: BackendCommandEvidence::BindingOpened(binding.clone()),
    }];
    if matches!(
        binding.continuation_strategy(),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            ..
        }
    ) {
        steps.push(BackendScriptStep::Emit(
            BackendEvent::ContextPolicyChanged {
                policy: ContextPolicyChanged::try_new(
                    1,
                    true,
                    ContextStrategy::PortableSummaryV1Alpha1,
                    85,
                    90,
                    Some(10),
                    Some(65_536),
                )
                .unwrap(),
            },
        ));
    }
    steps.extend([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: input.clone(),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request),
        },
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta("durable streamed answer".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: {
                let evidence = BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                    "codex.app-server/turn-outcome/v1",
                    "turn-a",
                ));
                match binding.continuation_strategy() {
                    ContinuationStrategy::BackendManagedState => evidence,
                    ContinuationStrategy::ExactReplay { .. } => {
                        evidence.with_replay(ModelReplayDelta::new(
                            Some(ModelReplayContract::new("system", Vec::new())),
                            vec![
                                input.model_replay_item(),
                                ModelReplayItem::Message {
                                    role: ModelReplayRole::Assistant,
                                    content: "durable streamed answer".to_owned(),
                                    refusal: None,
                                },
                            ],
                        ))
                    },
                }
            },
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let backend = ScriptedBackend::new(steps)
        .with_capabilities(BackendCapabilities::none().with_image_input(image_capability));
    let mut repository = MemoryRepository::default();
    let journal = SessionJournal::with_repository_and_descriptor(
        Box::new(repository.clone()),
        crate::fixture_descriptor(session_id),
    );
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime.initialize_durability();
    runtime
        .configure_input_admission(Box::new(TestInputAdmission))
        .unwrap();
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(AgentCommand::StartTurn { turn, input }, submission)
        .unwrap();
    loop {
        match runtime.poll_event().unwrap() {
            RuntimePoll::Event(AgentEvent::TurnFinished {
                turn: finished,
                outcome: crate::TurnOutcome::Completed,
            }) if finished == turn => break,
            RuntimePoll::Event(_) => {},
            RuntimePoll::Pending | RuntimePoll::Closed => {
                panic!("the fixture backend ended before publishing its resumable Turn")
            },
        }
    }
    runtime.shutdown().unwrap();
    let continuation = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    (repository, continuation)
}

fn exact_child_binding() -> BackendBindingEvidence {
    let source = binding();
    BackendBindingEvidence::new(
        source.backend_kind(),
        source.backend_version(),
        source.binding_identity().clone(),
        source.model_identity().clone(),
        BackendIdentity::new(source.session_locator().schema(), "independent-child"),
        source.continuation_strategy(),
    )
}
