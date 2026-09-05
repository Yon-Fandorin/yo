use std::{
    num::NonZeroU64,
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
        RepositoryError, RepositorySequence, SessionRepository, StoredSession, StoredSessionReader,
        StoredSessionSnapshot,
    },
};
use yo_tui::GlyphProfile;

use super::{Command, execution::show_from_reader};

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

fn usage_command(session_id: SessionId, glyph_profile: GlyphProfile) -> Command {
    Command {
        session_id,
        output: crate::command::output::OutputOptions {
            format: super::super::output::OutputFormat::Text,
            glyph_profile,
        },
    }
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

// 존재하지 않는 UUID의 최상위 Usage 조회는 새 저장소나 Session을 만들지 않고 기존
// direct history 경계의 정확한 not-found 실패를 그대로 반환합니다.
#[test]
fn archived_usage_reports_the_existing_not_found_failure() {
    let session_id = usage_session_id();
    let error = match show_from_reader(None, usage_command(session_id, GlyphProfile::Rich)) {
        Err(error) => error,
        Ok(_) => panic!("a missing read-only repository cannot contain the requested Session"),
    };

    assert_eq!(
        error.to_string(),
        format!("stored Session {session_id} was not found")
    );
}

// 완료된 사용량 영수증이 없는 오래된 durable Session도 성공하며, 공유 archived renderer의
// 명시적 빈 보고서를 stdout에 내고 관측되지 않은 토큰을 0으로 꾸미지 않습니다.
#[test]
fn archived_usage_succeeds_with_an_empty_shared_projection() {
    let session_id = usage_session_id();
    let repository = durable_usage_repository(None);
    let output = show_from_reader(
        Some(&repository),
        usage_command(session_id, GlyphProfile::Rich),
    )
    .unwrap();

    assert!(output.stdout.starts_with("Stored Session Usage\n"));
    assert!(output.stdout.contains("completed_receipts=0"));
    assert!(
        output
            .stdout
            .contains("No completed usage receipts are available.")
    );
    assert!(!output.stdout.contains("input=0"));
    assert!(output.diagnostics.is_empty());
}

// 완료된 durable 영수증은 CLI가 다시 해석하거나 집계하지 않고 공유 typed projection과
// archived renderer를 거치며, ASCII profile도 기존 marker 의미를 그대로 보존합니다.
#[test]
fn archived_usage_routes_successful_receipts_through_the_shared_renderer() {
    let session_id = usage_session_id();
    let repository = durable_usage_repository(Some(grok_receipt(serde_json::json!(3))));
    let output = show_from_reader(
        Some(&repository),
        usage_command(session_id, GlyphProfile::Ascii),
    )
    .unwrap();

    assert!(output.stdout.contains("completed_receipts=1"));
    assert!(output.stdout.contains("\ninput=3\n"));
    assert!(output.stdout.contains("* [01] grok"));
    assert!(!output.stdout.contains("• [01]"));
    assert!(output.diagnostics.is_empty());
}

// 알려진 schema의 malformed durable 영수증은 기존 projection 오류 문구와 activity 근거를
// 유지한 채 전체 조회를 닫고, 부분 Usage 보고서를 성공값으로 반환하지 않습니다.
#[test]
fn archived_usage_preserves_malformed_receipt_failure_evidence() {
    let session_id = usage_session_id();
    let repository =
        durable_usage_repository(Some(grok_receipt(serde_json::json!("not-a-number"))));
    let error = match show_from_reader(
        Some(&repository),
        usage_command(session_id, GlyphProfile::Rich),
    ) {
        Err(error) => error,
        Ok(_) => panic!("known malformed receipts must fail the whole Usage projection"),
    };
    let rendered = error.to_string();

    assert!(rendered.starts_with(
        "projecting stored Session history: projecting stored Session Usage failed: invalid "
    ));
    assert!(rendered.contains(GROK_USAGE_SCHEMA));
    assert!(rendered.contains("receipt for activity"));
    assert!(rendered.contains("input_tokens must be a non-negative integer"));
}
