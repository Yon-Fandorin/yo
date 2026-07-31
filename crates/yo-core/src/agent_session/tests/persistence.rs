use std::{fs, path::PathBuf, time::SystemTime};

use super::{
    super::{AgentIntent, AgentSession},
    support::{activity, session, turn},
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentCommand, BackendEvent, BackendScriptStep,
    ScriptedBackend, TurnOutcome, UserInput,
    journal::codec::JournalRecord,
    session_repository::{LocalSessionRepository, journal::JournalRepository},
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-agent-session-persistence-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// 실제 AgentSession worker가 command, streaming delta, replacement snapshot, 종료를
// 처리하면 local JSONL을 다시 열어도 최종 message revision과 Turn 완료를 복구해야 한다.
// 이는 codec 단위 테스트가 아니라 live worker에서 single-writer 저장 경로까지 잇는 검증이다.
#[test]
fn live_worker_persists_a_recoverable_session_journal() {
    let directory = TestDirectory::new();
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let first_turn = turn(1);
    let answer = activity(first_turn, 1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: answer,
            kind: ActivityKind::AgentMessage,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: answer,
            update: ActivityUpdate::TextDelta("draft".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: answer,
            update: ActivityUpdate::TextSnapshot("final".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: answer,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: first_turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app =
        AgentSession::start_cancellable_with_repository(backend, session(), repository, || false)
            .unwrap()
            .unwrap();

    app.dispatch(AgentIntent::Submit("inspect".to_owned()))
        .unwrap();
    app.wait_until_processed(1);
    app.wait_until_no_active_turn();
    app.shutdown().unwrap();

    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let recovered = JournalRepository::new(repository)
        .recover(session())
        .unwrap();
    assert!(recovered.recovery_commit().is_none());
    let terminal = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            JournalRecord::MessageEnded(terminal) => Some(terminal),
            _ => None,
        })
        .expect("the agent message has a durable terminal seal");
    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.final_segment().unwrap().text(), "final");
    assert!(matches!(
        recovered.records().last().unwrap().record(),
        JournalRecord::EventCommitted(crate::AgentEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        })
    ));
}
