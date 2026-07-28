use super::{activity, runtime_with_active_turn};
use crate::{
    ActivityKind, ActivityOutcome, AgentCommand, AgentEvent, BackendEvent, BackendScriptStep,
    RuntimeError, RuntimePoll, TurnOutcome,
};

// backend의 Activity 흐름이 runtime을 거쳐 같은 상관관계의 검증된 AgentEvent로 변환되는지
// 확인한다.
#[test]
fn applies_backend_events_through_the_semantic_engine() {
    let active_turn = super::turn(super::session(1), 1);
    let tool = activity(active_turn, 1);
    let file_change = activity(active_turn, 2);
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: tool,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: file_change,
            kind: ActivityKind::FileChange,
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: file_change,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Close,
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, observed_turn) = runtime_with_active_turn(steps);

    assert_eq!(observed_turn, active_turn);
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ToolCall,
        }) if activity == tool
    ));
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: tool,
            outcome: ActivityOutcome::Completed,
        })
    );
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::FileChange,
        }) if activity == file_change
    ));
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: file_change,
            outcome: ActivityOutcome::Completed,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        })
    );
    assert_eq!(runtime.active_turn(), None);
    assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Closed);
    runtime.shutdown().unwrap();
}

// backend가 정상적으로 shutdown되면 남아 있던 활성 Activity를 먼저 Interrupted로 닫고
// Turn도 Interrupted로 닫아 성공한 cleanup 뒤 semantic state가 남지 않는지 확인한다.
#[test]
fn successful_shutdown_interrupts_remaining_semantic_work() {
    let active_turn = super::turn(super::session(1), 1);
    let work = activity(active_turn, 1);
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    runtime.poll_event().unwrap();

    let terminal_events = runtime.shutdown().unwrap();

    assert_eq!(
        terminal_events,
        vec![
            AgentEvent::ActivityFinished {
                activity: work,
                outcome: ActivityOutcome::Interrupted,
            },
            AgentEvent::TurnFinished {
                turn: active_turn,
                outcome: TurnOutcome::Interrupted,
            },
        ]
    );
    assert_eq!(runtime.active_turn(), None);
    assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Closed);
    assert!(runtime.shutdown().unwrap().is_empty());
}

// 중단 명령 수락만으로 Turn을 닫지 않고 backend의 Activity/Turn 중단 event 순서대로 닫는지
// 확인한다.
#[test]
fn accepted_interrupt_waits_for_backend_completion() {
    let active_turn = super::turn(super::session(1), 1);
    let work = activity(active_turn, 1);
    let interrupt = AgentCommand::InterruptTurn { turn: active_turn };
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        }),
        BackendScriptStep::AcceptCommand(interrupt.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: work,
            outcome: ActivityOutcome::Interrupted,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { .. })
    ));

    let immediate = runtime.execute_command(interrupt).unwrap();

    assert!(immediate.is_empty());
    assert_eq!(runtime.active_turn(), Some(active_turn));
    assert_eq!(
        runtime
            .execute_command(AgentCommand::InterruptTurn { turn: active_turn })
            .unwrap_err(),
        RuntimeError::CommandRejected(crate::AgentRejection::InterruptAlreadyRequested {
            turn: active_turn,
        })
    );
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            outcome: ActivityOutcome::Interrupted,
            ..
        })
    ));
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            outcome: TurnOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(runtime.active_turn(), None);
    runtime.shutdown().unwrap();
}
