use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, AgentIntent, AgentSession, BackendBindingEvidence, BackendCommandEvidence,
    BackendEvent, BackendIdentity, BackendOutcomeEvidence, BackendRequestEvidence,
    BackendScriptStep, CommandAdmission, HostWorkspacePath, InputSubmission, ScriptedBackend,
    SessionDescriptor, SessionId, SubmissionId, TranscriptRecord, TurnId, TurnRef, UserInput,
    WorkspaceHostId,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, RepositoryEntry, RepositoryError,
        RepositorySequence, SessionRepository, StoredBindingCacheState, StoredBindingCloseReason,
        StoredBindingTransitionMode, StoredContinuationStrategy, StoredExchangeDirection,
        StoredExchangeKind, StoredReplayExecutor, StoredRequestDetailAvailability,
        StoredRequestTraceRecord, StoredSession, StoredSessionContinuity, StoredSessionHistory,
        StoredSessionReader, StoredSessionRecovery, StoredSessionSnapshot, read_stored_session,
    },
};

use super::{
    project_chat, project_transcript_parts,
    request::{
        cache_state_text, close_reason_text, continuation_strategy_text, detail_availability_text,
        exchange_direction_text, exchange_kind_text, project_parts, transition_mode_text,
    },
};
use crate::GlyphProfile;

fn history() -> (SessionDescriptor, Vec<TranscriptRecord>) {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let host: WorkspaceHostId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
    let descriptor = SessionDescriptor::for_session(
        session_id,
        host,
        HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap(),
    );
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    (
        descriptor,
        vec![
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
                turn,
                input: UserInput::new("질문"),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::AgentMessage,
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot("답변".to_owned()),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            }),
        ],
    )
}

#[derive(Clone, Default)]
struct MemoryRepository {
    entries: Arc<Mutex<Vec<RepositoryEntry>>>,
}

impl SessionRepository for MemoryRepository {
    fn append(
        &mut self,
        _session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
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

impl StoredSessionReader for MemoryRepository {
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        Ok(Vec::new())
    }

    fn read_session(
        &self,
        _session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        Ok(StoredSessionSnapshot::Present(
            self.entries.lock().unwrap().clone(),
        ))
    }

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        SessionRepository::read_after(self, session_id, sequence, limit)
    }
}

fn durable_request_history() -> StoredSessionHistory {
    let (descriptor, _) = history();
    let session_id = descriptor.session_id();
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let submission_id: SubmissionId = "10000000-0000-4000-8000-000000000031".parse().unwrap();
    let binding = BackendBindingEvidence::new(
        "codex-app-server",
        "codex_cli_rs/1.0.0",
        BackendIdentity::new("test.binding/v1", "binding-value"),
        BackendIdentity::new("test.model/v1", "model-value"),
        BackendIdentity::new("test.session/v1", "session-value"),
        yo_core::ContinuationStrategy::BackendManagedState,
    );
    let request = BackendRequestEvidence::new(
        "test.payload/v1",
        BackendIdentity::new("test.exchange/v1", "exchange\nvalue\u{1b}"),
        BackendIdentity::new("test.request/v1", "request-value"),
    );
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(binding),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: UserInput::new("inspect"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(request),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "test.outcome/v1",
                "outcome-value",
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let repository = MemoryRepository::default();
    let mut session = AgentSession::start_cancellable_with_repository(
        backend,
        descriptor,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    let mut admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            submission_id,
            UserInput::new("inspect"),
        )))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(
            Instant::now() < deadline,
            "fixture submission was not admitted"
        );
        thread::sleep(Duration::from_millis(1));
        admission = session.retry(pending).unwrap();
    }
    loop {
        let completed = session
            .transcript_reader()
            .read_after(None)
            .entries()
            .iter()
            .any(|entry| {
                matches!(
                    entry.record(),
                    TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn: seen, .. })
                        if *seen == turn
                )
            });
        if completed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture backend did not complete its resumable Turn"
        );
        let _ = session.poll().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    session.shutdown().unwrap();
    read_stored_session(&repository, session_id).unwrap()
}

// 같은 저장 history를 Chat으로 보면 사용자/에이전트 본문만 간결하게 나오고 저장 schema나
// 물리 record 진단은 섞이지 않아 stdout을 그대로 읽거나 pipe로 넘길 수 있다.
#[test]
fn archived_chat_reuses_the_plain_chat_projection() {
    let (_, records) = history();
    let output = project_chat(&records, GlyphProfile::Rich).unwrap();

    assert!(output.contains("질문"));
    assert!(output.contains("답변"));
    assert!(!output.contains("journal_cutoff"));
}

// Transcript view는 live reader의 "metadata unavailable" 문구를 재사용하지 않고, 저장
// 스냅샷에서 실제로 검증한 cutoff/recovery/discovery와 각 semantic record를 함께 보여준다.
#[test]
fn archived_transcript_labels_the_durable_observation_boundary() {
    let (descriptor, records) = history();
    let output = project_transcript_parts(
        &descriptor,
        None,
        StoredSessionRecovery::NotRequired,
        StoredSessionContinuity::NotObservable,
        true,
        &records,
    );

    assert!(output.contains("journal_cutoff=descriptor-only"));
    assert!(output.contains("message_recovery=not-required"));
    assert!(output.contains("durability_continuity=not-observable"));
    assert!(output.contains("discovery=consistent"));
    assert!(output.contains("[#001] command.start_turn"));
    assert!(!output.contains("metadata, and Request Audit detail are unavailable"));
}

// 실제 durable 복구에서 얻은 다섯 상관 record와 명시적 close record를 합쳐 여섯
// 종류가 Journal 순서로 출력되고, payload·제어문자는 노출되지 않는지 검증합니다.
#[test]
fn archived_request_formats_every_correlation_record_in_journal_order() {
    let history = durable_request_history();
    assert_eq!(history.request_trace().len(), 5);
    let close_sequence = history.request_trace().last().unwrap().sequence().get() + 1;
    let closed = StoredRequestTraceRecord::BindingClosed {
        epoch: 1,
        reason: StoredBindingCloseReason::Exhausted,
    };
    let output = project_parts(
        history.descriptor(),
        history.journal_cutoff(),
        history.recovery(),
        history.continuity(),
        history.discovery_consistent(),
        history
            .request_trace()
            .iter()
            .map(|entry| (entry.sequence().get(), entry.record()))
            .chain(std::iter::once((close_sequence, &closed))),
    );

    assert!(output.starts_with("Stored Session Request diagnostic\n"));
    assert!(output.contains("observation_boundary=validated-session-journal-correlation"));
    assert!(output.contains("request_audit_detail=unavailable(reason=no-audit-reader)"));
    assert!(output.contains("message_recovery=not-required"));
    assert!(output.contains("durability_continuity=not-observable"));
    assert!(output.contains("discovery=consistent"));
    let names = [
        "binding.opened",
        "exchange.observed",
        "request.accepted",
        "outcome.resumable",
        "continuation.anchor",
        "binding.closed",
    ];
    let positions = names.map(|name| output.find(name).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(output.contains("backend_kind=\"codex-app-server\""));
    assert!(output.contains("continuation_strategy=backend-managed-state"));
    assert!(output.contains("kind=request"));
    assert!(output.contains("correlation_sequence=none"));
    assert!(output.contains("exchange_identity.value=\"exchange\\nvalue\\u{1b}\""));
    assert!(output.contains("detail_availability=unpersisted"));
    assert!(output.contains("request_identity.value=\"request-value\""));
    assert!(output.contains("outcome_identity.value=\"outcome-value\""));
    assert!(output.contains("replay_delta_sequence=none"));
    assert!(output.contains("journal_boundary="));
    assert!(output.contains("reason=exhausted"));
    assert!(!output.contains("input="));
}

// 공개 core enum의 모든 닫힌 값은 Request stdout에서 안정적인 소문자 진단어로
// 표시되어, 비슷한 variant를 잘못 연결하거나 새 값을 조용히 빠뜨리지 못합니다.
#[test]
fn archived_request_labels_every_closed_diagnostic_value() {
    assert_eq!(
        [
            StoredContinuationStrategy::ExactReplay {
                executor: StoredReplayExecutor::LocalClient,
            },
            StoredContinuationStrategy::ExactReplay {
                executor: StoredReplayExecutor::ManagedServer,
            },
            StoredContinuationStrategy::BackendManagedState,
        ]
        .map(continuation_strategy_text),
        [
            "exact-replay(local-client)",
            "exact-replay(managed-server)",
            "backend-managed-state",
        ]
    );
    assert_eq!(
        [
            StoredExchangeKind::Request,
            StoredExchangeKind::Response,
            StoredExchangeKind::Notification,
            StoredExchangeKind::ServerRequest,
            StoredExchangeKind::Retry,
            StoredExchangeKind::TerminalOutcome,
        ]
        .map(exchange_kind_text),
        [
            "request",
            "response",
            "notification",
            "server-request",
            "retry",
            "terminal-outcome",
        ]
    );
    assert_eq!(
        [
            StoredExchangeDirection::YoToBackend,
            StoredExchangeDirection::BackendToYo,
        ]
        .map(exchange_direction_text),
        ["yo-to-backend", "backend-to-yo"]
    );
    assert_eq!(
        [
            StoredRequestDetailAvailability::Persisted,
            StoredRequestDetailAvailability::Volatile,
            StoredRequestDetailAvailability::Missing,
            StoredRequestDetailAvailability::Unsupported,
            StoredRequestDetailAvailability::Unpersisted,
            StoredRequestDetailAvailability::Redacted,
        ]
        .map(detail_availability_text),
        [
            "persisted",
            "volatile",
            "missing",
            "unsupported",
            "unpersisted",
            "redacted",
        ]
    );
    assert_eq!(
        [
            StoredBindingTransitionMode::Initial,
            StoredBindingTransitionMode::ExactReplay,
            StoredBindingTransitionMode::LossyHandoff,
        ]
        .map(transition_mode_text),
        ["initial", "exact-replay", "lossy-handoff"]
    );
    assert_eq!(
        [
            StoredBindingCacheState::NotApplicable,
            StoredBindingCacheState::Lost,
            StoredBindingCacheState::Unknown,
        ]
        .map(cache_state_text),
        ["not-applicable", "lost", "unknown"]
    );
    assert_eq!(
        [
            StoredBindingCloseReason::Replaced,
            StoredBindingCloseReason::Revoked,
            StoredBindingCloseReason::Exhausted,
        ]
        .map(close_reason_text),
        ["replaced", "revoked", "exhausted"]
    );
}
