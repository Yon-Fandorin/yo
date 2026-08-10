use std::task::{Context, Poll};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentCommand, AgentRuntime, BackendEvent,
    BackendScriptStep, RuntimeError, RuntimePoll, ScriptedBackend, SubmissionId, TranscriptRecord,
    TurnOutcome, UserInput,
};

use super::{activity, turn};
use crate::{
    appearance::AppearanceState,
    runner::{
        AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch, state::TuiState,
        unix::apply_agent_poll,
    },
    surface::{CellContent, Point, Size},
};

struct RuntimeConnection {
    runtime: AgentRuntime<ScriptedBackend>,
}

impl AgentConnection for RuntimeConnection {
    type Error = RuntimeError;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        unreachable!("this projection test consumes only backend observations")
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        unreachable!("this projection test consumes only backend observations")
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(match self.runtime.poll_event()? {
            RuntimePoll::Pending => AgentPoll::Pending,
            RuntimePoll::Event(event) => AgentPoll::Record(TranscriptRecord::EventCommitted(event)),
            RuntimePoll::Closed => AgentPoll::Closed,
        })
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

// ScriptedBackend의 coding Activity가 core 상관관계 검증을 통과한 뒤 agent observation을
// 하나씩 적용하는 TUI 경계에서 Tool과 FileChange transcript로 함께 투영되는지 확인한다.
#[test]
fn projects_fake_backend_coding_activities_through_core_into_tui() {
    let active_turn = turn();
    let tool = activity(1);
    let file = activity(2);
    let create = AgentCommand::CreateSession {
        session_id: active_turn.session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::from("inspect"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: tool,
            update: ActivityUpdate::TextSnapshot("$ cargo test\nok".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: tool,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: file,
            kind: ActivityKind::FileChange,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: file,
            update: ActivityUpdate::TextSnapshot("update: src/lib.rs".to_owned()),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: file,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    let mut state = TuiState::new();
    for event in runtime.execute_command(create).unwrap() {
        state.observe(event).unwrap();
    }
    for event in runtime
        .execute_submission(start, SubmissionId::new().unwrap())
        .unwrap()
    {
        state.observe(event).unwrap();
    }
    let mut connection = RuntimeConnection { runtime };

    let mut observed = 0;
    loop {
        let observation = connection.poll().unwrap();
        if matches!(observation, AgentPoll::Pending) {
            break;
        }
        assert!(apply_agent_poll(&mut state, observation).unwrap());
        observed += 1;
        assert!(observed < 32, "scripted runtime must converge to pending");
    }
    assert!(observed > 0);

    let frame = state
        .prepare_frame(Size::new(32, 18), &AppearanceState::default().pin())
        .unwrap();
    let rows = (0..18)
        .map(|y| {
            (0..32)
                .map(
                    |x| match frame.surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank | CellContent::Continuation { .. } => ' ',
                        CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
                    },
                )
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rows.contains("Running tool…"));
    assert!(rows.contains("$ cargo test"));
    assert!(rows.contains("File change observed"));
    assert!(rows.contains("update: src/lib.rs"));
}
