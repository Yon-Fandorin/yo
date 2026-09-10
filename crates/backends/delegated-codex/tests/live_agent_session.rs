use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use yo_backend_delegated_codex::{CodexBackend, CodexBackendConfig};
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, AgentIntent, AgentSession,
    AgentSessionPoll, CommandAdmission, HostWorkspacePath, SessionDescriptor, SubmissionOutcome,
    TranscriptRecord, TurnOutcome, TurnRef, WorkspaceHostId,
    session_repository::{
        LocalSessionRepository, SessionRepository, SessionWriterRepository,
        recover_stored_session_continuation,
    },
};

// 호환되는 로컬 Codex와 인증 환경이 있을 때 실제 도구가 disposable workspace에 파일을
// 만들고 Tool, FileChange, 완료 Turn event가 모두 관찰되는지 환경 통합 경로로 확인한다.
#[test]
#[ignore = "requires compatible authenticated Codex and performs one model turn"]
fn local_codex_completes_a_real_file_change() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let workspace =
        env::temp_dir().join(format!("yo-codex-file-change-{}-{unique}", process::id()));
    fs::create_dir(&workspace).unwrap();

    let result = run_local_file_change(&workspace);
    let cleanup = fs::remove_dir_all(&workspace);

    result.unwrap();
    cleanup.unwrap();
}

fn run_local_file_change(workspace: &Path) -> Result<(), String> {
    let backend = CodexBackend::spawn(CodexBackendConfig::new(workspace))
        .map_err(|error| error.to_string())?;
    let mut app = AgentSession::start(backend).map_err(|error| error.to_string())?;
    let transcript = app.transcript_reader();
    let mut cursor = None;
    let mut admission = app.dispatch(
        AgentIntent::submit(
            "First run `pwd` with the shell command tool and wait for it to complete. Then use the \
         file patch tool to create yo-proof.txt in the current workspace containing exactly \
         YO_CODEX_INTEGRATION_OK followed by one newline. Perform both actions, then stop.",
        )
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let admission_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match admission {
            CommandAdmission::Queued => break,
            CommandAdmission::Backpressured(pending) if Instant::now() < admission_deadline => {
                thread::sleep(Duration::from_millis(10));
                admission = app.retry(pending).map_err(|error| error.to_string())?;
            },
            CommandAdmission::Backpressured(_) => {
                app.shutdown().map_err(|error| error.to_string())?;
                return Err("coding probe was not admitted within 30 seconds".into());
            },
            CommandAdmission::Rejected { rejection, .. } => {
                app.shutdown().map_err(|error| error.to_string())?;
                return Err(format!(
                    "coding probe admission was rejected: {rejection:?}"
                ));
            },
        }
    }

    let deadline = Instant::now() + Duration::from_secs(180);
    let mut activities = HashMap::new();
    let mut payloads = HashMap::<_, String>::new();
    let mut completed_tool = false;
    let mut completed_file_change = false;
    let turn_outcome = 'turn: loop {
        if Instant::now() >= deadline {
            break Err(format!(
                "Codex Turn did not complete within 180 seconds (observed {} activities; completed tool: {completed_tool}; completed file change: {completed_file_change})",
                activities.len()
            ));
        }
        match app.poll().map_err(|error| error.to_string())? {
            AgentSessionPoll::Pending => thread::sleep(Duration::from_millis(10)),
            AgentSessionPoll::Closed => {
                break Err("Codex closed before the Turn completed".to_owned());
            },
            AgentSessionPoll::Changed => {
                let slice = transcript.read_after(cursor);
                if let Some(last) = slice.entries().last() {
                    cursor = Some(last.sequence());
                }
                for entry in slice.entries() {
                    let TranscriptRecord::EventCommitted(event) = entry.record() else {
                        continue;
                    };
                    match event {
                        AgentEvent::ActivityStarted { activity, kind } => {
                            activities.insert(*activity, *kind);
                            if matches!(
                                kind,
                                ActivityKind::ApprovalRequest { .. }
                                    | ActivityKind::UserInputRequest { .. }
                            ) {
                                break 'turn Err("the bounded coding probe requires an unexpected interactive request; no approval was sent".to_owned());
                            }
                        },
                        AgentEvent::ActivityFinished { activity, outcome } => {
                            if *outcome == ActivityOutcome::Completed {
                                match activities.get(activity) {
                                    Some(ActivityKind::ToolCall) => completed_tool = true,
                                    Some(ActivityKind::FileChange) => completed_file_change = true,
                                    _ => {},
                                }
                            }
                        },
                        AgentEvent::TurnFinished { outcome, .. } => {
                            break 'turn Ok(outcome.clone());
                        },
                        AgentEvent::ActivityUpdated { activity, update } => match update {
                            ActivityUpdate::TextSnapshot(text) => {
                                payloads.insert(*activity, text.clone());
                            },
                            ActivityUpdate::TextDelta(text) => {
                                payloads.entry(*activity).or_default().push_str(text)
                            },
                        },
                        AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {},
                    }
                }
            },
        }
    };
    let shutdown = app.shutdown().map_err(|error| error.to_string());
    let turn_outcome = turn_outcome?;
    shutdown?;

    if turn_outcome != TurnOutcome::Completed {
        return Err(format!("Codex Turn ended as {turn_outcome:?}"));
    }
    if !completed_tool {
        return Err("no completed Tool Activity was observed".to_owned());
    }
    if !completed_file_change {
        return Err("no completed FileChange Activity was observed".to_owned());
    }
    if !payloads.iter().any(|(activity, text)| {
        activities.get(activity) == Some(&ActivityKind::FileChange)
            && text.contains("+YO_CODEX_INTEGRATION_OK")
    }) {
        return Err(format!(
            "file-change payloads: {:?}",
            payloads
                .iter()
                .filter(|(activity, _)| activities.get(activity) == Some(&ActivityKind::FileChange))
                .map(|(_, text)| text.chars().take(2000).collect::<String>())
                .collect::<Vec<_>>()
        ));
    }
    let content =
        fs::read_to_string(workspace.join("yo-proof.txt")).map_err(|error| error.to_string())?;
    if content != "YO_CODEX_INTEGRATION_OK\n" {
        return Err(format!("unexpected file content: {content:?}"));
    }
    Ok(())
}

// 실제 Codex process를 종료한 뒤 durable continuation으로 다시 열어 같은 Session과
// locator, 증가한 Turn ID 및 기존 기록을 보존하면서 이전 입력의 nonce를 기억하는지 검증한다.
#[test]
#[ignore = "requires compatible authenticated Codex and performs two text-only model turns"]
fn local_codex_resumes_a_durable_session_and_remembers_prior_input() {
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = env::temp_dir().join(format!(
        "yo-codex-durable-resume-{}-{}",
        process::id(),
        WorkspaceHostId::new().unwrap(),
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    run_local_durable_resume(&fixture.0).unwrap();
    fs::remove_dir_all(&fixture.0).unwrap();
}

fn run_local_durable_resume(root: &Path) -> Result<(), String> {
    let workspace = root.join("workspace");
    let storage = root.join("repository");
    fs::create_dir(&workspace).map_err(|error| error.to_string())?;
    let host = WorkspaceHostId::new().map_err(|error| error.to_string())?;
    let descriptor = SessionDescriptor::new(
        host,
        HostWorkspacePath::normalize_local(&workspace).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let session_id = descriptor.session_id();
    let nonce = format!(
        "YO_NONCE_{}",
        WorkspaceHostId::new().map_err(|error| error.to_string())?
    );
    let config = CodexBackendConfig::new(&workspace)
        .with_read_only_review(true)
        .with_request_timeout(Duration::from_secs(30));
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    repository
        .acquire_session_writer(session_id)
        .map_err(|error| error.to_string())?;
    let backend = CodexBackend::spawn(config.clone()).map_err(|error| error.to_string())?;
    let start_deadline = Instant::now() + Duration::from_secs(45);
    let mut app = AgentSession::start_cancellable_with_repository(
        backend,
        descriptor.clone(),
        repository,
        || Instant::now() >= start_deadline,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "initial Session startup exceeded 45 seconds".to_owned())?;
    let first = run_text_only_turn(
        &mut app,
        "initial",
        &format!(
            "Remember this exact nonce in our conversation: {nonce}. Do not call tools, read or write files, or ask questions. Reply only ACK. I will ask you to recall it later."
        ),
    );
    let cleanup = app.shutdown().map_err(|error| error.to_string());
    let (first_turn, first_reply) = first?;
    cleanup?;
    drop(app);
    if first_turn.session_id() != session_id
        || first_turn.turn_id().get().get() != 1
        || first_reply.trim() != "ACK"
    {
        return Err(
            "first text-only Turn did not produce ACK at the initial durable Turn coordinate"
                .to_owned(),
        );
    }

    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let prefix = repository
        .read_after(session_id, None, 4096)
        .map_err(|error| error.to_string())?;
    if prefix.is_empty() || prefix.len() == 4096 {
        return Err("first durable prefix is empty or exceeds the probe bound".to_owned());
    }
    let continuation = recover_stored_session_continuation(&mut repository, session_id)
        .map_err(|error| error.to_string())?;
    if continuation.descriptor() != &descriptor || continuation.target().session_id() != session_id
    {
        return Err("continuation changed the saved Session descriptor".to_owned());
    }
    let binding = continuation.target().binding().clone();
    let backend = CodexBackend::spawn(config).map_err(|error| error.to_string())?;
    let resume_deadline = Instant::now() + Duration::from_secs(45);
    let mut app = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository,
        || Instant::now() >= resume_deadline,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "durable Session resume exceeded 45 seconds".to_owned())?;
    // This request deliberately contains no nonce or prior input text.
    let second = run_text_only_turn(
        &mut app,
        "resumed",
        "What exact nonce did I ask you to remember earlier in this conversation? Reply with only that nonce, with no formatting or explanation. Do not call tools, read or write files, or ask questions.",
    );
    let cleanup = app.shutdown().map_err(|error| error.to_string());
    let (second_turn, second_reply) = second?;
    cleanup?;
    drop(app);
    if second_turn.session_id() != session_id
        || second_turn.turn_id().get().get() != first_turn.turn_id().get().get() + 1
    {
        return Err("resumed Turn did not advance within the same Session".to_owned());
    }
    if second_reply != nonce {
        return Err("resumed Codex did not return the exact remembered nonce".to_owned());
    }
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    let final_records = repository
        .read_after(session_id, None, 8192)
        .map_err(|error| error.to_string())?;
    if final_records.len() <= prefix.len() || !final_records.starts_with(&prefix) {
        return Err(
            "resume did not append while preserving the exact prior durable prefix".to_owned(),
        );
    }
    let final_continuation = recover_stored_session_continuation(&mut repository, session_id)
        .map_err(|error| error.to_string())?;
    if final_continuation.descriptor() != &descriptor
        || !binding.same_resume_identity(final_continuation.target().binding())
        || binding.session_locator() != final_continuation.target().binding().session_locator()
    {
        return Err("resume changed the durable Session or backend locator/binding".to_owned());
    }
    if fs::read_dir(&workspace)
        .map_err(|error| error.to_string())?
        .next()
        .is_some()
    {
        return Err("text-only probe unexpectedly created workspace files".to_owned());
    }
    Ok(())
}

fn run_text_only_turn(
    app: &mut AgentSession,
    stage: &'static str,
    prompt: &str,
) -> Result<(TurnRef, String), String> {
    let transcript = app.transcript_reader();
    // A resumed reader already contains the previous conversation; inspect new records only.
    let mut cursor = transcript.read_after(None).head();
    eprintln!("Codex durable resume probe: stage={stage}, dispatching text-only Turn");
    let mut admission = app
        .dispatch(AgentIntent::submit(prompt).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let admission_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match admission {
            CommandAdmission::Queued => break,
            CommandAdmission::Backpressured(pending) if Instant::now() < admission_deadline => {
                thread::sleep(Duration::from_millis(10));
                admission = app.retry(pending).map_err(|error| error.to_string())?;
            },
            _ => {
                return Err(format!(
                    "stage={stage}: text-only probe admission was rejected or timed out"
                ));
            },
        }
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut report_at = Instant::now() + Duration::from_secs(30);
    let mut submission_status = "queued";
    let mut record_count = 0_u64;
    let mut event_count = 0_u64;
    let mut activity_count = 0_u64;
    let mut update_count = 0_u64;
    let mut changed_count = 0_u64;
    let mut started_turn = None;
    let mut messages = HashMap::new();
    let mut completed_messages = Vec::new();
    loop {
        if Instant::now() >= report_at {
            let turn_id = started_turn.map(|turn: TurnRef| turn.turn_id().get().get());
            let diagnostic = format!(
                "stage={stage}, submission={submission_status}, started_turn={turn_id:?}, records={record_count}, events={event_count}, activities={activity_count}, text_updates={update_count}, messages={}, completed_messages={}, changed_polls={changed_count}, durability={:?}",
                messages.len(),
                completed_messages.len(),
                transcript.durability()
            );
            if Instant::now() >= deadline {
                return Err(format!(
                    "text-only Codex Turn exceeded 180 seconds ({diagnostic})"
                ));
            }
            eprintln!("Codex durable resume probe: {diagnostic}");
            report_at = (Instant::now() + Duration::from_secs(30)).min(deadline);
        }
        if let Some(outcome) = app.take_submission_outcome() {
            match outcome {
                SubmissionOutcome::Accepted { .. } => submission_status = "accepted",
                SubmissionOutcome::Rejected { rejection, .. } => {
                    return Err(format!(
                        "stage={stage}: text-only input admission rejected ({:?})",
                        rejection.kind()
                    ));
                },
            }
        }
        match app.poll().map_err(|_| {
            format!("stage={stage}: AgentSession poll failed (provider payload withheld)")
        })? {
            AgentSessionPoll::Closed => {
                return Err(format!(
                    "stage={stage}: Codex closed before text-only Turn completion"
                ));
            },
            AgentSessionPoll::Changed => changed_count += 1,
            AgentSessionPoll::Pending => {},
        }
        let slice = transcript.read_after(cursor);
        if let Some(last) = slice.entries().last() {
            cursor = Some(last.sequence());
        }
        for entry in slice.entries() {
            record_count += 1;
            let TranscriptRecord::EventCommitted(event) = entry.record() else {
                continue;
            };
            event_count += 1;
            match event {
                AgentEvent::TurnStarted { turn } => {
                    if started_turn.replace(*turn).is_some() {
                        return Err("text-only probe observed multiple Turn starts".to_owned());
                    }
                },
                AgentEvent::ActivityStarted { activity, kind } => {
                    activity_count += 1;
                    match kind {
                        ActivityKind::AgentMessage => {
                            messages.insert(*activity, String::new());
                        },
                        ActivityKind::ModelWork => {},
                        _ => {
                            return Err(format!(
                                "stage={stage}: text-only probe observed an unexpected {kind:?} Activity; no approval was sent"
                            ));
                        },
                    }
                },
                AgentEvent::ActivityUpdated { activity, update } => {
                    update_count += 1;
                    if let Some(message) = messages.get_mut(activity) {
                        match update {
                            ActivityUpdate::TextSnapshot(text) => message.clone_from(text),
                            ActivityUpdate::TextDelta(text) => message.push_str(text),
                        }
                    }
                },
                AgentEvent::ActivityFinished { activity, outcome }
                    if messages.contains_key(activity) =>
                {
                    if *outcome != ActivityOutcome::Completed {
                        return Err("text-only response Activity did not complete".to_owned());
                    }
                    completed_messages.push(*activity);
                },
                AgentEvent::TurnFinished { turn, outcome } => {
                    if Some(*turn) != started_turn
                        || *outcome != TurnOutcome::Completed
                        || completed_messages.len() != 1
                    {
                        return Err(
                            "text-only Turn must complete once with one final message".to_owned()
                        );
                    }
                    eprintln!(
                        "Codex durable resume probe: stage={stage}, completed_turn={}, events={event_count}, submission={submission_status}",
                        turn.turn_id().get().get()
                    );
                    return Ok((*turn, messages.remove(&completed_messages[0]).unwrap()));
                },
                _ => {},
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}
