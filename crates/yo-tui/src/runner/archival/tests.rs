use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, AgentIntent, AgentSession, BackendBindingEvidence, BackendCommandEvidence,
    BackendEvent, BackendIdentity, BackendOutcomeEvidence, BackendRequestEvidence,
    BackendScriptStep, CommandAdmission, Failure, HostWorkspacePath, InputSubmission,
    ScriptedBackend, SessionDescriptor, SessionId, SubmissionId, TranscriptRecord, TurnId,
    TurnOutcome, TurnRef, UserInput, WorkspaceHostId,
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
    ArchivedContentPolicy, ArchivedProjectionOptions, ArchivedSessionView,
    project_archived_session, project_archived_session_with_options, project_archived_usage,
    project_chat, project_transcript_parts, project_transcript_parts_with_options,
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

// 기존 진입점과 새 options 진입점의 기본값은 같은 durable history를 한 바이트도
// 다르게 렌더링하지 않아, CLI가 새 API로 이동하기 전후의 stdout 호환성을 지킨다.
#[test]
fn archived_transcript_default_options_preserve_legacy_output_exactly() {
    let history = durable_request_history();

    let legacy = project_archived_session(
        &history,
        ArchivedSessionView::Transcript,
        GlyphProfile::Ascii,
    )
    .unwrap();
    let with_options = project_archived_session_with_options(
        &history,
        ArchivedSessionView::Transcript,
        GlyphProfile::Ascii,
        ArchivedProjectionOptions::default(),
    )
    .unwrap();

    assert_eq!(with_options, legacy);
}

// limit=2는 전체 다섯 semantic record 중 최신 두 개를 먼저 고른 뒤 시간순으로
// 출력하며, 선택된 record의 표시는 새 번호가 아니라 원래 #004와 #005를 유지한다.
#[test]
fn archived_transcript_limit_selects_the_newest_records_with_original_numbers() {
    let (descriptor, records) = history();
    let output = project_transcript_parts_with_options(
        &descriptor,
        None,
        StoredSessionRecovery::NotRequired,
        StoredSessionContinuity::NotObservable,
        true,
        &records,
        ArchivedProjectionOptions::new(NonZeroUsize::new(2), ArchivedContentPolicy::Full),
    );

    assert!(!output.contains("[#003]"));
    let fourth = output.find("[#004] event.activity_updated").unwrap();
    let fifth = output.find("[#005] event.activity_finished").unwrap();
    assert!(fourth < fifth);
}

// content=none은 사용자 입력, Activity update, Activity/Turn 실패 본문을 전혀
// 노출하지 않고 각 본문의 의미 타입과 원래 UTF-8 byte 길이만 남긴다.
#[test]
fn archived_transcript_none_exposes_only_content_type_and_byte_length() {
    let (descriptor, mut records) = history();
    let turn = match &records[0] {
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { turn, .. }) => *turn,
        record => panic!("unexpected fixture record: {record:?}"),
    };
    let activity = match &records[3] {
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, .. }) => *activity,
        record => panic!("unexpected fixture record: {record:?}"),
    };
    let input_secret = "INPUT_SECRET_가";
    let update_secret = "UPDATE_SECRET_나";
    let activity_failure_secret = "ACTIVITY_FAILURE_SECRET_다";
    let turn_failure_secret = "TURN_FAILURE_SECRET_라";
    records[0] = TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
        turn,
        input: UserInput::new(input_secret),
    });
    records[3] = TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextDelta(update_secret.to_owned()),
    });
    records[4] = TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
        activity,
        outcome: ActivityOutcome::Failed(Failure::new(activity_failure_secret)),
    });
    records.push(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
        turn,
        outcome: TurnOutcome::Failed(Failure::new(turn_failure_secret)),
    }));

    let output = project_transcript_parts_with_options(
        &descriptor,
        None,
        StoredSessionRecovery::NotRequired,
        StoredSessionContinuity::NotObservable,
        true,
        &records,
        ArchivedProjectionOptions::new(None, ArchivedContentPolicy::None),
    );

    for secret in [
        input_secret,
        update_secret,
        activity_failure_secret,
        turn_failure_secret,
    ] {
        assert!(!output.contains(secret));
    }
    for (content_type, byte_length) in [
        ("user_input", input_secret.len()),
        ("activity_text_delta", update_secret.len()),
        ("activity_failure_message", activity_failure_secret.len()),
        ("turn_failure_message", turn_failure_secret.len()),
    ] {
        assert!(output.contains(&format!(
            "content.type={content_type}\ncontent.utf8_bytes={byte_length}"
        )));
    }
    assert!(!output.contains("content.preview="));
}

// 1MB를 넘는 단일 Unicode 입력도 content=preview 출력은 고정 크기로 제한되고,
// UTF-8 경계를 자르지 않으며 preview 뒤의 비밀 문자열을 stdout에 포함하지 않는다.
#[test]
fn archived_transcript_preview_bounds_one_very_large_unicode_record() {
    let (descriptor, records) = history();
    let turn = match &records[0] {
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { turn, .. }) => *turn,
        record => panic!("unexpected fixture record: {record:?}"),
    };
    let secret = "TAIL_SECRET_MUST_NOT_APPEAR";
    let mut input = "가".repeat(400_000);
    input.push_str(secret);
    let input_bytes = input.len();
    let records = [TranscriptRecord::CommandCommitted(
        AgentCommand::StartTurn {
            turn,
            input: UserInput::new(input),
        },
    )];

    let output = project_transcript_parts_with_options(
        &descriptor,
        None,
        StoredSessionRecovery::NotRequired,
        StoredSessionContinuity::NotObservable,
        true,
        &records,
        ArchivedProjectionOptions::new(None, ArchivedContentPolicy::Preview),
    );

    assert!(
        output.len() < 2_000,
        "bounded output was {} bytes",
        output.len()
    );
    assert!(output.contains(&format!("content.utf8_bytes={input_bytes}")));
    assert!(output.contains(&format!("content.preview={:?}", "가".repeat(85))));
    assert!(output.contains("content.preview_truncated=true"));
    assert!(!output.contains(secret));
}

// 256-byte 경계에 걸친 결합 문자와 ZWJ emoji는 일부 code point만 preview에
// 들어가지 않고, 예산 안에서 완결된 마지막 extended grapheme까지만 출력된다.
#[test]
fn archived_transcript_preview_preserves_combining_and_zwj_graphemes() {
    let (descriptor, records) = history();
    let turn = match &records[0] {
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { turn, .. }) => *turn,
        record => panic!("unexpected fixture record: {record:?}"),
    };
    let cases = [("a".repeat(255), "e\u{301}"), ("b".repeat(250), "👩‍💻")];

    for (complete_prefix, crossing_grapheme) in cases {
        let input = format!("{complete_prefix}{crossing_grapheme}TAIL");
        let records = [TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn,
                input: UserInput::new(input),
            },
        )];
        let output = project_transcript_parts_with_options(
            &descriptor,
            None,
            StoredSessionRecovery::NotRequired,
            StoredSessionContinuity::NotObservable,
            true,
            &records,
            ArchivedProjectionOptions::new(None, ArchivedContentPolicy::Preview),
        );

        assert!(output.contains(&format!("content.preview={complete_prefix:?}")));
        assert!(!output.contains(crossing_grapheme));
        assert!(output.contains("content.preview_truncated=true"));
    }
}

// Transcript 이외의 view는 불완전한 lifecycle이나 진단 trace를 만들 수 있으므로
// 새 진입점이 비기본 bound를 조용히 무시하지 않고 명시적인 오류로 거부한다.
#[test]
fn archived_projection_rejects_non_default_bounds_for_other_views() {
    let history = durable_request_history();
    let options = ArchivedProjectionOptions::new(None, ArchivedContentPolicy::Preview);

    for view in [ArchivedSessionView::Chat, ArchivedSessionView::Request] {
        let error =
            project_archived_session_with_options(&history, view, GlyphProfile::Ascii, options)
                .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("archived projection bounds are supported only for Transcript, not {view:?}")
        );
    }
}

// 독립 Usage 진입점은 동일한 StoredSessionHistory의 typed usage projection을 통해
// 성공적으로 빈 보고서를 만들며, Chat/Transcript/Request view routing과 분리된다.
#[test]
fn archived_usage_has_a_direct_plain_renderer_entrypoint() {
    let history = durable_request_history();
    let output = project_archived_usage(&history, GlyphProfile::Ascii).unwrap();

    assert!(output.starts_with("Stored Session Usage\n"));
    assert!(output.contains("completed_receipts=0"));
    assert!(output.contains("No completed usage receipts are available."));
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
