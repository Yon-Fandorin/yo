use std::sync::{Arc, Mutex};

use super::{
    super::{
        ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentBackend,
        AgentCommand, ApprovalDecision, BackendEvent, BackendPoll, ModelConnectorEvent,
        NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
        ToolApprovalRequirement, ToolExecution, ToolExecutionHost, ToolExecutionOutcome,
        ToolExecutionPoll, ToolExecutionRequest, TurnOutcome,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, backend, binding, completed,
        context_profile, drain_until_turn, event_rounds, registry, turn,
    },
};
use crate::{ToolExecutionError, ToolExecutionResult, ToolId, UserInput};

struct PendingHost {
    cancelled: Arc<Mutex<usize>>,
    shutdowns: Arc<Mutex<usize>>,
}

impl ToolExecutionHost for PendingHost {
    fn identity(&self) -> &str {
        "pending-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        Ok(Box::new(PendingExecution {
            cancelled: Arc::clone(&self.cancelled),
            shutdowns: Arc::clone(&self.shutdowns),
        }))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

struct PendingExecution {
    cancelled: Arc<Mutex<usize>>,
    shutdowns: Arc<Mutex<usize>>,
}

struct CleanupFailingHost {
    cancelled: Arc<Mutex<usize>>,
    shutdowns: Arc<Mutex<usize>>,
}

impl ToolExecutionHost for CleanupFailingHost {
    fn identity(&self) -> &str {
        "cleanup-failing-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        Ok(Box::new(CleanupFailingExecution {
            result: Some(ToolExecutionResult::new(
                ToolExecutionOutcome::Completed,
                "result",
                false,
            )),
            cancelled: Arc::clone(&self.cancelled),
            shutdowns: Arc::clone(&self.shutdowns),
        }))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

struct CleanupFailingExecution {
    result: Option<ToolExecutionResult>,
    cancelled: Arc<Mutex<usize>>,
    shutdowns: Arc<Mutex<usize>>,
}

impl ToolExecution for CleanupFailingExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        Ok(ToolExecutionPoll::Ready)
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.result.take()
    }

    fn cancel(&self) {
        *self.cancelled.lock().unwrap() += 1;
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        *self.shutdowns.lock().unwrap() += 1;
        Err(ToolExecutionError::new("executor join failed"))
    }
}

impl ToolExecution for PendingExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        Ok(ToolExecutionPoll::Pending)
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        None
    }

    fn cancel(&self) {
        *self.cancelled.lock().unwrap() += 1;
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        *self.shutdowns.lock().unwrap() += 1;
        Ok(())
    }
}

// 승인이 필요한 도구는 정확한 요청 응답 전에는 실행되지 않고 승인 후 한 번만 실행된다.
#[test]
fn native_backend_required_approval_gates_tool_execution() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r1".to_owned(),
                },
                ModelConnectorEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                },
                ModelConnectorEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"README.md"}"#.to_owned(),
                },
                completed("r1"),
            ],
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r2".to_owned(),
                },
                ModelConnectorEvent::TextDelta {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                    content_index: 0,
                    delta: "ok".to_owned(),
                },
                ModelConnectorEvent::MessageDone {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                },
                completed("r2"),
            ],
        ],
        ToolApprovalRequirement::Required,
        Arc::clone(&starts),
    );
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();

    let request = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            }) => break ActivityRequestRef::new(activity, request_id),
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            other => panic!("unexpected poll before approval: {other:?}"),
        }
    };
    assert_eq!(*starts.lock().unwrap(), 0);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();
    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::ResumableTurnFinished { .. }
    ));
    assert_eq!(*starts.lock().unwrap(), 1);
}

// 활성 executor를 중단하면 취소와 정리를 각각 요청하고 불확실한 effect를 interrupted ToolResult로
// 봉인한다.
#[test]
fn native_backend_interrupts_and_seals_an_active_tool_execution() {
    let cancelled = Arc::new(Mutex::new(0));
    let shutdowns = Arc::new(Mutex::new(0));
    let rounds = vec![vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        completed("r1"),
    ]];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(PendingHost {
                cancelled: Arc::clone(&cancelled),
                shutdowns: Arc::clone(&shutdowns),
            }),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();

    for _ in 0..100 {
        let _ = backend.poll_event().unwrap();
        if backend
            .turn
            .as_ref()
            .is_some_and(|state| state.active_tool.is_some())
        {
            break;
        }
    }
    assert!(
        backend
            .turn
            .as_ref()
            .is_some_and(|state| state.active_tool.is_some())
    );
    backend
        .execute_command(AgentCommand::InterruptTurn { turn: turn() })
        .unwrap();

    assert_eq!(*cancelled.lock().unwrap(), 1);
    assert_eq!(*shutdowns.lock().unwrap(), 1);
    let mut saw_interrupted_tool = false;
    loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityFinished {
                outcome: ActivityOutcome::Interrupted,
                ..
            }) => saw_interrupted_tool = true,
            BackendPoll::Event(BackendEvent::TurnFinished {
                outcome: TurnOutcome::Interrupted,
                ..
            }) => break,
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before interruption was sealed"),
        }
    }
    assert!(saw_interrupted_tool);
}

// executor join 실패는 effect를 재시도하지 않고 cancel과 마지막 cleanup을 시도한 뒤,
// cleanup 진단을 포함한 실패 Turn과 닫힌 ToolResult Activity로 봉인한다.
#[test]
fn native_backend_seals_the_turn_when_tool_cleanup_fails() {
    let cancelled = Arc::new(Mutex::new(0));
    let shutdowns = Arc::new(Mutex::new(0));
    let rounds = vec![vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        completed("r1"),
    ]];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(CleanupFailingHost {
                cancelled: Arc::clone(&cancelled),
                shutdowns: Arc::clone(&shutdowns),
            }),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("cleanup ambiguity must fail the Turn")
    };
    assert!(failure.message().contains("tool execution cleanup failed"));
    assert_eq!(*cancelled.lock().unwrap(), 1);
    assert_eq!(*shutdowns.lock().unwrap(), 2);
}
