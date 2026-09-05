use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, AgentIntent, AgentSession, BackendEvent, BackendScriptStep, CommandAdmission,
    HostWorkspacePath, InputSubmission, ScriptedBackend, SessionDescriptor, SessionId,
    SubmissionId, TranscriptRecord, TurnId, TurnOutcome, TurnRef, UserInput, WorkspaceHostId,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, GROK_USAGE_SCHEMA, RepositoryEntry,
        RepositoryError, RepositorySequence, SessionRepository, StoredDiscoveryMismatch,
        StoredDiscoveryValidation, StoredSession, StoredSessionContinuity, StoredSessionReader,
        StoredSessionSnapshot,
    },
};

use super::{
    super::{Command as SessionCommand, Content as SessionContent, View as SessionView},
    archival_diagnostics, read_only_resume_from, show_from_reader,
};
use crate::command::output::{OutputFormat, OutputOptions};

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

fn usage_session_id() -> SessionId {
    "01890f00-0000-7000-8000-000000000001".parse().unwrap()
}

fn durable_usage_repository(receipt: Option<String>) -> MemoryRepository {
    let session_id = usage_session_id();
    let host: WorkspaceHostId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
    let descriptor = SessionDescriptor::for_session(
        session_id,
        host,
        HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap(),
    );
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let has_receipt = receipt.is_some();
    let mut steps = vec![BackendScriptStep::AcceptCommand(
        AgentCommand::CreateSession { session_id },
    )];
    if let Some(receipt) = receipt {
        steps.extend([
            BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
                turn,
                input: UserInput::new("inspect"),
            }),
            BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }),
            BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(receipt),
            }),
            BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            }),
            BackendScriptStep::Emit(BackendEvent::TurnFinished {
                turn,
                outcome: TurnOutcome::Completed,
            }),
        ]);
    }
    steps.push(BackendScriptStep::Shutdown(Ok(())));

    let repository = MemoryRepository::default();
    let mut session = AgentSession::start_cancellable_with_repository(
        ScriptedBackend::new(steps),
        descriptor,
        repository.clone(),
        || false,
    )
    .unwrap()
    .unwrap();
    if has_receipt {
        let submission_id: SubmissionId = "10000000-0000-4000-8000-000000000031".parse().unwrap();
        let mut admission = session
            .dispatch(AgentIntent::Submit(InputSubmission::new(
                submission_id,
                UserInput::new("inspect"),
            )))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while let CommandAdmission::Backpressured(pending) = admission {
            assert!(
                deadline > Instant::now(),
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
                        TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                            turn: seen,
                            ..
                        }) if *seen == turn
                    )
                });
            if completed {
                break;
            }
            assert!(deadline > Instant::now(), "fixture Turn did not complete");
            let _ = session.poll().unwrap();
            thread::sleep(Duration::from_millis(1));
        }
    }
    session.shutdown().unwrap();
    repository
}

fn grok_receipt(input_tokens: serde_json::Value) -> String {
    serde_json::json!({
        "schema": GROK_USAGE_SCHEMA,
        "source_profile": "grok.acp.prompt-response.usage/v1",
        "prompt_request_id": 42,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": 2,
            "total_tokens": 5,
            "reasoning_tokens": 0,
            "cache_read_input_tokens": 1,
            "cache_write_input_tokens": 0
        }
    })
    .to_string()
}

// 기본 Chat stdout은 pipe 가능한 본문만 유지하되 v1이 volatile suffix 부재를 증명하지
// 못한다는 경계는 stderr 진단으로 노출하고, 같은 사실을 본문에 적는 Transcript는 중복하지 않습니다.
#[test]
fn chat_warns_when_durability_continuity_is_not_observable() {
    let session_id = "01890f00-0000-7000-8000-000000000001".parse().unwrap();

    let chat = archival_diagnostics(
        session_id,
        SessionView::Chat,
        StoredSessionContinuity::NotObservable,
        StoredDiscoveryValidation::Consistent,
    );
    let transcript = archival_diagnostics(
        session_id,
        SessionView::Transcript,
        StoredSessionContinuity::NotObservable,
        StoredDiscoveryValidation::Consistent,
    );
    let request = archival_diagnostics(
        session_id,
        SessionView::Request,
        StoredSessionContinuity::NotObservable,
        StoredDiscoveryValidation::Consistent,
    );

    assert_eq!(chat.len(), 1);
    assert!(chat[0].message().contains("volatile suffix"));
    assert!(transcript.is_empty());
    assert!(request.is_empty());
}

// Chat의 continuity 경고와 discovery 불일치는 서로 다른 복구 단서이므로 함께 남고,
// core가 만든 typed 원인과 physical 위치도 CLI stderr 경계에서 유실되지 않습니다.
#[test]
fn chat_preserves_continuity_and_typed_discovery_diagnostics_together() {
    let session_id = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let mismatch = StoredDiscoveryMismatch::new(
        RepositorySequence::new(10),
        yo_core::session_repository::StoredDiscoveryMismatchKind::BindingEpoch { claimed: 4 },
    );

    let diagnostics = archival_diagnostics(
        session_id,
        SessionView::Chat,
        StoredSessionContinuity::NotObservable,
        StoredDiscoveryValidation::Mismatch(mismatch),
    );

    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics[0].message().contains("volatile suffix"));
    assert!(diagnostics[1].message().contains("binding epoch 4"));
    assert!(diagnostics[1].message().contains("repository sequence 10"));
}

// CLI Session 경계는 Transcript 전용 selector를 공유 Projection options로 그대로 넘겨 최신
// 세 record만 원래 번호로 남기고, 그 안의 usage JSON payload는 none 정책으로 숨긴다.
#[test]
fn bounded_transcript_options_reach_the_archived_projection() {
    let session_id = usage_session_id();
    let repository = durable_usage_repository(Some(grok_receipt(serde_json::json!(3))));
    let output = show_from_reader(
        Some(&repository),
        session_id,
        SessionCommand {
            session_id: Some(session_id),
            all: false,
            details: false,
            view: SessionView::Transcript,
            output: OutputOptions {
                format: OutputFormat::Text,
                glyph_profile: yo_tui::GlyphProfile::Rich,
            },
            limit: NonZeroUsize::new(3),
            content: Some(SessionContent::None),
        },
    )
    .unwrap();

    assert_eq!(output.stdout.matches("[#").count(), 3);
    assert!(
        output
            .stdout
            .contains("content.type=activity_text_snapshot")
    );
    assert!(!output.stdout.contains(GROK_USAGE_SCHEMA));
}

// read-only resume는 application이 preflight에서 캡처한 reader를 그대로 사용해야 하며,
// alternate repository가 존재해도 command session은 supplied reader만 관측해야 합니다.
#[test]
fn read_only_resume_projects_history_from_the_supplied_reader() {
    let session_id = usage_session_id();
    let captured = durable_usage_repository(Some("captured history A".to_owned()));
    let alternate = durable_usage_repository(Some("alternate history B".to_owned()));

    let alternate_output = read_only_resume_from(
        &alternate,
        session_id,
        yo_tui::GlyphProfile::Rich,
        "alternate",
    )
    .unwrap();
    assert!(alternate_output.stdout.contains("alternate history B"));

    let output = read_only_resume_from(
        &captured,
        session_id,
        yo_tui::GlyphProfile::Rich,
        "captured preflight failure",
    )
    .unwrap();

    assert!(output.stdout.contains("captured history A"));
    assert!(!output.stdout.contains("alternate history B"));
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message().contains("captured preflight failure"))
    );
}
