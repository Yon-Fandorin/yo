use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use yo_backend_delegated_codex::{CodexBackend, CodexBackendConfig};
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityQuestion, ActivityRequestRef, ActivityUpdate,
    AgentEvent, AgentIntent, AgentSession, AgentSessionPoll, CommandAdmission, HostWorkspacePath,
    JournalDurability, SecretInput, SessionDescriptor, SubmissionOutcome, TranscriptEntry,
    TranscriptReader, TranscriptRecord, TurnOutcome, TurnRef, WorkspaceHostId,
    session_repository::{
        LocalSessionReader, LocalSessionRepository, SessionRepository, SessionWriterRepository,
        read_stored_session, recover_stored_session_continuation,
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

// 실제 Codex가 동적 진단 도구를 선택한 뒤 입력값을 모델·저장소에 남기지 않고
// 고정 완료 상태만 받는지, 한 번의 모델 Turn으로 확인한다.
#[test]
#[ignore = "requires Codex 0.155.1, authentication, YO_CODEX_SECRET_ENTRY_PROBE=1, and performs one model turn"]
fn local_codex_dynamic_probe_discards_sample_secret() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("yo-codex-secret-probe-{}-{unique}", process::id()));
    fs::create_dir(&root).unwrap();

    let result = run_local_secret_probe(&root);
    let cleanup = fs::remove_dir_all(&root);

    result.unwrap();
    cleanup.unwrap();
}

fn run_local_secret_probe(root: &Path) -> Result<(), String> {
    const LOCAL_RECEIPT: &str = "Sample secret entry verified locally and discarded.";
    const PROBE_RESULT: &str = "Sample secret entry verified locally and discarded by Yo. No value was sent to Codex or the model.";
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
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024)
        .map_err(|error| error.to_string())?;
    repository
        .acquire_session_writer(session_id)
        .map_err(|error| error.to_string())?;
    let backend = CodexBackend::spawn(
        CodexBackendConfig::new(&workspace).with_request_timeout(Duration::from_secs(30)),
    )
    .map_err(|error| error.to_string())?;
    let start_deadline = Instant::now() + Duration::from_secs(45);
    let mut app =
        AgentSession::start_cancellable_with_repository(backend, descriptor, repository, || {
            Instant::now() >= start_deadline
        })
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "secret-probe Session startup exceeded 45 seconds".to_owned())?;
    let transcript = app.transcript_reader();
    let sample = format!(
        "YO_CODEX_SAMPLE_ONLY_{}",
        WorkspaceHostId::new().map_err(|error| error.to_string())?
    );
    let mut cursor = None;
    let mut probe = SecretProbeRun::default();
    let mut admission = app
        .dispatch(
            AgentIntent::submit(
                "Call yo_secret_entry_probe exactly once now with exactly an empty object. Do not call any other tool or ask any other question. After its one result returns, do not call it again; reply only YO_CODEX_SECRET_PROBE_DONE.",
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
                let _ = app.shutdown();
                return Err("secret-probe Turn was not admitted within 30 seconds".to_owned());
            },
            CommandAdmission::Rejected { rejection, .. } => {
                let _ = app.shutdown();
                return Err(format!("secret-probe Turn was rejected: {rejection:?}"));
            },
        }
    }

    let deadline = Instant::now() + Duration::from_secs(180);
    let outcome = 'turn: loop {
        if Instant::now() >= deadline {
            break Err("secret-probe Codex Turn exceeded 180 seconds".to_owned());
        }
        match app.poll().map_err(|error| error.to_string())? {
            AgentSessionPoll::Pending => thread::sleep(Duration::from_millis(10)),
            AgentSessionPoll::Closed => {
                break Err("Codex closed before secret-probe Turn completed".to_owned());
            },
            AgentSessionPoll::Changed => {
                for entry in drain_transcript(&transcript, &mut cursor) {
                    let TranscriptRecord::EventCommitted(event) = entry.record() else {
                        continue;
                    };
                    match event {
                        AgentEvent::ActivityStarted { activity, kind } => {
                            if let Err(error) = probe.started(*activity, *kind) {
                                break 'turn Err(error);
                            }
                        },
                        AgentEvent::ActivityUpdated { activity, update } => {
                            let text = match probe.updated(*activity, update, &sample) {
                                Ok(text) => text,
                                Err(error) => break 'turn Err(error),
                            };
                            if probe.ready_to_submit(*activity, &text) {
                                let response = app
                                    .dispatch(AgentIntent::RespondToSecretInput {
                                        request: probe
                                            .input_request
                                            .expect("ready probe input retains its request"),
                                        input: SecretInput::new(sample.clone())
                                            .map_err(|error| error.to_string())?,
                                    })
                                    .map_err(|error| error.to_string())?;
                                if response != CommandAdmission::Queued {
                                    break 'turn Err(
                                        "secret-probe input response was not immediately queued"
                                            .to_owned(),
                                    );
                                }
                                probe.submitted = true;
                            }
                        },
                        AgentEvent::ActivityFinished { activity, outcome } => {
                            if let Err(error) = probe.finished(*activity, outcome) {
                                break 'turn Err(error);
                            }
                        },
                        AgentEvent::TurnFinished { outcome, .. } => break 'turn Ok(outcome.clone()),
                        AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {},
                    }
                }
            },
        }
    };
    let shutdown = app.shutdown().map_err(|error| error.to_string());
    let outcome = outcome?;
    shutdown?;
    drop(app);

    if outcome != TurnOutcome::Completed {
        return Err(format!("secret-probe Turn ended as {outcome:?}"));
    }
    probe.complete(PROBE_RESULT, "YO_CODEX_SECRET_PROBE_DONE")?;
    if !matches!(transcript.durability(), JournalDurability::Durable { .. }) {
        return Err("secret-probe Session was not durable after the completed Turn".to_owned());
    }
    let final_records = drain_transcript(&transcript, &mut cursor);
    if !final_records.is_empty() {
        return Err("secret-probe transcript changed after Turn completion".to_owned());
    }
    let durable = fs::read_dir(&storage)
        .map_err(|error| error.to_string())?
        .try_fold(Vec::new(), |mut bytes, entry| {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                bytes.extend(fs::read(entry.path()).map_err(|error| error.to_string())?);
            }
            Ok::<_, String>(bytes)
        })?;
    if durable
        .windows(sample.len())
        .any(|window| window == sample.as_bytes())
    {
        return Err("durable Session storage retained the sample value".to_owned());
    }
    let mut all_records_cursor = None;
    let transcript_debug = format!(
        "{:?}",
        drain_transcript(&transcript, &mut all_records_cursor)
    );
    if transcript_debug.contains(&sample) {
        return Err("Yo transcript retained the sample value".to_owned());
    }
    let reader = LocalSessionReader::open(&storage).map_err(|error| error.to_string())?;
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("durable secret-probe read failed: {error}"))?;
    let persisted = history.records();
    if format!("{persisted:?}").contains(&sample) {
        return Err("reopened durable Session retained the sample value".to_owned());
    }
    if !persisted.iter().any(|record| {
        matches!(record, TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update: ActivityUpdate::TextSnapshot(text), .. }) if text == LOCAL_RECEIPT)
    }) {
        return Err("reopened durable Session omitted the payload-free probe receipt".to_owned());
    }
    if !persisted.iter().any(|record| {
        matches!(
            record,
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Completed,
                ..
            })
        )
    }) {
        return Err("reopened durable Session omitted the completed probe Turn".to_owned());
    }
    Ok(())
}

#[derive(Default)]
struct SecretProbeRun {
    activities: HashMap<yo_core::ActivityRef, ActivityKind>,
    texts: HashMap<yo_core::ActivityRef, String>,
    tool_calls: Vec<yo_core::ActivityRef>,
    probe_tool: Option<yo_core::ActivityRef>,
    input_request: Option<ActivityRequestRef>,
    input_snapshot: bool,
    submitted: bool,
    probe_tool_completed: bool,
    input_response_completed: bool,
}

impl SecretProbeRun {
    fn started(
        &mut self,
        activity: yo_core::ActivityRef,
        kind: ActivityKind,
    ) -> Result<(), String> {
        match kind {
            ActivityKind::ToolCall => self.tool_calls.push(activity),
            ActivityKind::ToolResult
            | ActivityKind::FileChange
            | ActivityKind::ApprovalRequest { .. }
            | ActivityKind::ApprovalResponse { .. } => {
                return Err(format!("secret-probe Turn started an unexpected {kind:?}"));
            },
            ActivityKind::UserInputRequest { request_id } => {
                if self.probe_tool.is_none()
                    || self
                        .input_request
                        .replace(ActivityRequestRef::new(activity, request_id))
                        .is_some()
                {
                    return Err(
                        "secret-probe input was not uniquely preceded by its tool call".to_owned(),
                    );
                }
            },
            ActivityKind::UserInputResponse { .. } => {
                if !self.submitted {
                    return Err("secret-probe input response started before submission".to_owned());
                }
            },
            ActivityKind::ModelWork | ActivityKind::AgentMessage => {},
        }
        self.activities.insert(activity, kind);
        Ok(())
    }

    fn updated(
        &mut self,
        activity: yo_core::ActivityRef,
        update: &ActivityUpdate,
        sample: &str,
    ) -> Result<String, String> {
        let text = self.texts.entry(activity).or_default();
        match update {
            ActivityUpdate::TextSnapshot(snapshot) => *text = snapshot.clone(),
            ActivityUpdate::TextDelta(delta) => text.push_str(delta),
        }
        if text.contains(sample) {
            return Err("model-facing activity text retained the sample value".to_owned());
        }
        if self.activities.get(&activity) == Some(&ActivityKind::ToolCall) {
            if !text.contains("yo_secret_entry_probe") {
                return Err(format!(
                    "secret-probe Turn started an unexpected tool: {}",
                    text.chars().take(240).collect::<String>()
                ));
            }
            if self
                .probe_tool
                .replace(activity)
                .is_some_and(|tool| tool != activity)
            {
                return Err("secret-probe Turn started the probe tool more than once".to_owned());
            }
        }
        Ok(text.clone())
    }

    fn ready_to_submit(&mut self, activity: yo_core::ActivityRef, text: &str) -> bool {
        if self.submitted || Some(activity) != self.input_request.map(|request| request.activity())
        {
            return false;
        }
        let Some(question) = ActivityQuestion::from_snapshot(text) else {
            return false;
        };
        self.input_snapshot = question.is_secret;
        self.input_snapshot
            && self.probe_tool.is_some_and(|tool| {
                self.texts
                    .get(&tool)
                    .is_some_and(|snapshot| snapshot.contains("yo_secret_entry_probe"))
            })
    }

    fn finished(
        &mut self,
        activity: yo_core::ActivityRef,
        outcome: &ActivityOutcome,
    ) -> Result<(), String> {
        if Some(activity) == self.probe_tool {
            if *outcome != ActivityOutcome::Completed {
                return Err(format!("secret-probe tool did not complete: {outcome:?}"));
            }
            self.probe_tool_completed = true;
        }
        if matches!(
            self.activities.get(&activity),
            Some(ActivityKind::UserInputResponse { .. })
        ) {
            if *outcome != ActivityOutcome::Completed {
                return Err(format!(
                    "secret-probe input response did not complete: {outcome:?}"
                ));
            }
            self.input_response_completed = true;
        }
        Ok(())
    }

    fn complete(&self, result: &str, final_reply: &str) -> Result<(), String> {
        let Some(tool) = self.probe_tool else {
            return Err("Codex did not start the secret-entry probe tool".to_owned());
        };
        if self.tool_calls != [tool] {
            return Err("secret-probe Turn started an unexpected tool".to_owned());
        }
        if !self.input_snapshot
            || !self.submitted
            || !self.probe_tool_completed
            || !self.input_response_completed
        {
            return Err("secret-probe tool/input lifecycle did not complete".to_owned());
        }
        if !self
            .texts
            .get(&tool)
            .is_some_and(|text| text.contains("yo_secret_entry_probe"))
        {
            return Err("Codex probe tool call did not expose its exact tool identity".to_owned());
        }
        if !self
            .texts
            .get(&tool)
            .is_some_and(|text| text.contains(result))
        {
            return Err(
                "Codex did not report the fixed probe result on the completed tool".to_owned(),
            );
        }
        let replies = self
            .activities
            .iter()
            .filter(|(_, kind)| **kind == ActivityKind::AgentMessage)
            .filter_map(|(activity, _)| self.texts.get(activity))
            .collect::<Vec<_>>();
        if replies.len() != 1 || replies[0].trim() != final_reply {
            return Err("Codex did not finish after the completed probe tool result".to_owned());
        }
        Ok(())
    }
}

fn drain_transcript(
    transcript: &TranscriptReader,
    cursor: &mut Option<yo_core::JournalSequence>,
) -> Vec<TranscriptEntry> {
    let mut records = Vec::new();
    loop {
        let slice = transcript.read_after(*cursor);
        let head = slice.head();
        let entries = slice.into_entries();
        let Some(last) = entries.last() else {
            return records;
        };
        *cursor = Some(last.sequence());
        records.extend(entries);
        if *cursor == head {
            return records;
        }
    }
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
