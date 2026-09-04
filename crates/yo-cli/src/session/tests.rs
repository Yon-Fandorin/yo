use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, AgentIntent, AgentSession, BackendEvent, BackendScriptStep, CommandAdmission,
    HostWorkspacePath, InputSubmission, ScriptedBackend, SessionDescriptor, SubmissionId,
    TranscriptRecord, TurnId, TurnOutcome, TurnRef, UserInput, WorkspaceHostId,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, GROK_USAGE_SCHEMA, RepositoryEntry,
        RepositoryError, RepositorySequence, SessionRepository, StoredSession,
        StoredSessionSnapshot,
    },
};

use super::*;
use crate::command::{OutputFormat, OutputOptions, UsageCommand};

fn unbounded(rows: &[SessionRow], all: bool, details: bool) -> String {
    format_rows(
        rows,
        all,
        details,
        OutputWidth::Unbounded,
        HeadingStyle::Plain,
    )
    .unwrap()
}

fn row(resume: &str, workspace: &str) -> SessionRow {
    SessionRow {
        resume: resume.to_owned(),
        status: "available".to_owned(),
        workspace: workspace.to_owned(),
        updated: "1700000002000".to_owned(),
        started: "1700000001000".to_owned(),
        version: "v1".to_owned(),
        continuation: "unavailable".to_owned(),
        path: format!("/work/{workspace}"),
        detail: String::new(),
    }
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

fn usage_session_id() -> SessionId {
    "01890f00-0000-7000-8000-000000000001".parse().unwrap()
}

fn usage_command(session_id: SessionId, glyph_profile: yo_tui::GlyphProfile) -> UsageCommand {
    UsageCommand {
        session_id,
        output: OutputOptions {
            format: OutputFormat::Text,
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

// 기본 목록은 사용자가 바로 고를 UUID와 상태/시간만 보여주고, 현재 workspace와 중복되는
// WORKSPACE 및 schema 세부사항은 넣지 않아 짧은 terminal에서도 핵심 열이 유지된다.
#[test]
fn ordinary_list_keeps_the_compact_column_order() {
    let output = unbounded(&[row("session-a", "yo")], false, false);
    let header = output.lines().next().unwrap();

    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        ["RESUME", "STATUS", "UPDATED", "STARTED"]
    );
    assert!(!output.contains("WORKSPACE"));
    assert!(!output.contains("VERSION"));
}

// `--all --details`는 다른 workspace를 구분할 짧은 WORKSPACE를 날짜보다 앞에 두고,
// 검수용 schema/continuation/full path/reason을 뒤에 확장하되 UUID 열은 그대로 유지한다.
#[test]
fn all_details_expands_metadata_without_replacing_the_resume_identity() {
    let output = unbounded(&[row("session-a", "yo")], true, true);
    let header = output.lines().next().unwrap();

    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        [
            "RESUME",
            "STATUS",
            "WORKSPACE",
            "UPDATED",
            "STARTED",
            "VERSION",
            "CONTINUATION",
            "PATH",
            "DETAIL",
        ]
    );
    assert!(output.contains("/work/yo"));
}

// Session이 하나도 없는 새 머신은 설명 문장을 stdout 데이터처럼 출력하지 않고 빈 성공
// 결과를 반환해 `yo session | ...` 파이프가 실제 row만 다룰 수 있게 한다.
#[test]
fn empty_list_has_empty_stdout() {
    assert_eq!(unbounded(&[], false, false), "");
}

// stdout이 terminal이면 측정한 폭을 사용하고, 측정 실패 시에도 80셀로 복구하지만,
// 파이프 출력은 폭과 무관한 한 줄 형식을 유지해 shell 조합의 결과가 안정적입니다.
#[test]
fn output_width_policy_distinguishes_terminals_from_pipes() {
    let observed = NonZeroU16::new(120).unwrap();

    assert_eq!(
        output_width(true, Ok(observed)),
        OutputWidth::Bounded(observed)
    );
    assert_eq!(
        output_width(true, Err(std::io::Error::other("unavailable"))),
        OutputWidth::Bounded(NonZeroU16::new(80).unwrap())
    );
    assert_eq!(output_width(false, Ok(observed)), OutputWidth::Unbounded);
    assert_eq!(heading_style(true), HeadingStyle::BoldAnsi);
    assert_eq!(heading_style(false), HeadingStyle::Plain);
}

// 상세 목록이 terminal 폭을 넘으면 PATH와 DETAIL을 함께 표 아래로 옮기되, 각
// label/value pair가 전체 폭에 들어가면 독립된 한 줄에서 불필요한 개행 없이 읽습니다.
#[test]
fn narrow_details_fold_path_and_detail_below_the_primary_row() {
    let mut value = row("session-a", "yo");
    value.detail = "reason".to_owned();

    let output = format_rows(
        &[value],
        true,
        true,
        OutputWidth::Bounded(NonZeroU16::new(80).unwrap()),
        HeadingStyle::Plain,
    )
    .unwrap();

    assert!(output.contains("PATH  /work/yo\n"));
    assert!(output.contains("DETAIL  reason\n"));
}

// 저장소 오류의 제어문자는 표 밖으로 cursor를 움직이지 않고 읽을 수 있는 escape로
// 바뀌어, 상세 목록의 한 row가 다른 row나 terminal 상태를 손상하지 않습니다.
#[test]
fn table_diagnostics_escape_control_characters() {
    assert_eq!(terminal_safe("bad\npath\u{1b}"), "bad\\npath\\u{1b}");
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
        yo_core::session_repository::RepositorySequence::new(10),
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

// 존재하지 않는 UUID의 최상위 Usage 조회는 새 저장소나 Session을 만들지 않고 기존
// direct history 경계의 정확한 not-found 실패를 그대로 반환합니다.
#[test]
fn archived_usage_reports_the_existing_not_found_failure() {
    let session_id = usage_session_id();
    let error = match crate::usage::show_from_reader(
        None,
        usage_command(session_id, yo_tui::GlyphProfile::Rich),
    ) {
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
    let output = crate::usage::show_from_reader(
        Some(&repository),
        usage_command(session_id, yo_tui::GlyphProfile::Rich),
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
    let output = crate::usage::show_from_reader(
        Some(&repository),
        usage_command(session_id, yo_tui::GlyphProfile::Ascii),
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
    let error = match crate::usage::show_from_reader(
        Some(&repository),
        usage_command(session_id, yo_tui::GlyphProfile::Rich),
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
