use std::{
    collections::HashMap,
    fs, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::super::{AgentIntent, AgentSession};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, AgentEvent, AgentSessionPoll,
    ApprovalDecision, CodexBackend, CodexBackendConfig, TranscriptRecord, TurnOutcome,
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
    let workspace = std::env::temp_dir().join(format!(
        "yo-codex-file-change-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&workspace).unwrap();

    let result = run_local_file_change(&workspace);
    let cleanup = fs::remove_dir_all(&workspace);

    result.unwrap();
    cleanup.unwrap();
}

fn run_local_file_change(workspace: &std::path::Path) -> Result<(), String> {
    let backend = CodexBackend::spawn(CodexBackendConfig::new(workspace))
        .map_err(|error| error.to_string())?;
    let mut app = AgentSession::start(backend).map_err(|error| error.to_string())?;
    let transcript = app.transcript_reader();
    let mut cursor = None;
    app.dispatch(AgentIntent::Submit(
        "First run `pwd` with the shell command tool and wait for it to complete. Then use the \
         file patch tool to create yo-proof.txt in the current workspace containing exactly \
         YO_CODEX_INTEGRATION_OK followed by one newline. Perform both actions, then stop."
            .to_owned(),
    ))
    .map_err(|error| error.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(180);
    let mut activities = HashMap::new();
    let mut completed_tool = false;
    let mut completed_file_change = false;
    let turn_outcome = 'turn: loop {
        if Instant::now() >= deadline {
            break Err("Codex Turn did not complete within 180 seconds".to_owned());
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
                            if let ActivityKind::ApprovalRequest { request_id } = kind {
                                app.dispatch(AgentIntent::RespondToApproval {
                                    request: ActivityRequestRef::new(*activity, *request_id),
                                    decision: ApprovalDecision::Approved,
                                })
                                .map_err(|error| error.to_string())?;
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
                        AgentEvent::SessionCreated { .. }
                        | AgentEvent::TurnStarted { .. }
                        | AgentEvent::ActivityUpdated { .. } => {},
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
    let content =
        fs::read_to_string(workspace.join("yo-proof.txt")).map_err(|error| error.to_string())?;
    if content != "YO_CODEX_INTEGRATION_OK\n" {
        return Err(format!("unexpected file content: {content:?}"));
    }
    Ok(())
}
