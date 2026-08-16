use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendCommandEvidence, BackendEvent, ContinuationStrategy,
        ModelConnectorEvent, ModelConnectorInputItem, ModelReplayItem, ModelReplayRole,
        NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices, ReplayExecutor,
        ToolApprovalRequirement, ToolExecution, ToolExecutionHost, ToolExecutionOutcome,
        ToolExecutionRequest, TurnOutcome,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockExecution, backend, binding,
        completed, context_profile, drain_until_turn, event_rounds, registry, turn,
    },
};
use crate::{
    AgentEvent, AgentRuntime, RuntimePoll, SubmissionId, ToolExecutionError, ToolExecutionResult,
    ToolId, UserInput,
};

struct OrderedHost {
    starts: Arc<Mutex<Vec<String>>>,
}

struct DeadlineHost {
    observed: Arc<Mutex<Vec<Option<Duration>>>>,
}

impl ToolExecutionHost for DeadlineHost {
    fn identity(&self) -> &str {
        "deadline-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        self.observed
            .lock()
            .unwrap()
            .push(request.absolute_execution_timeout);
        Ok(Box::new(MockExecution {
            result: Some(ToolExecutionResult::new(
                ToolExecutionOutcome::Completed,
                r#"{"contents":"ok"}"#,
                false,
            )),
        }))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

impl ToolExecutionHost for OrderedHost {
    fn identity(&self) -> &str {
        "ordered-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        self.starts
            .lock()
            .unwrap()
            .push(request.call.call_id().to_owned());
        Ok(Box::new(MockExecution {
            result: Some(ToolExecutionResult::new(
                ToolExecutionOutcome::Completed,
                r#"{"contents":"ok"}"#,
                false,
            )),
        }))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

// 모델 함수 호출은 검증·단일 실행·결과 기록을 거친 뒤 다음 모델 라운드로 이어진다.
#[test]
fn native_backend_runs_automatic_tool_once_and_replays_it_before_the_next_round() {
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
                    delta: "완".to_owned(),
                },
                ModelConnectorEvent::TextDelta {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                    content_index: 0,
                    delta: "료".to_owned(),
                },
                ModelConnectorEvent::MessageDone {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                },
                completed("r2"),
            ],
        ],
        ToolApprovalRequirement::Automatic,
        Arc::clone(&starts),
    );
    let binding_evidence = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    assert!(matches!(
        binding_evidence,
        BackendCommandEvidence::BindingOpened(ref evidence)
            if evidence.continuation_strategy()
                == ContinuationStrategy::ExactReplay {
                    executor: ReplayExecutor::LocalClient,
                    replay_profile: crate::ReplayProfile::SemanticOnly,
                }
    ));
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();

    let BackendEvent::ResumableTurnFinished { evidence, .. } = drain_until_turn(&mut backend)
    else {
        panic!("the deterministic model loop must finish resumably")
    };
    assert_eq!(*starts.lock().unwrap(), 1);
    let items = evidence.model_replay().unwrap().items();
    assert!(matches!(
        items[0],
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            ..
        }
    ));
    assert!(matches!(items[1], ModelReplayItem::FunctionCall { .. }));
    assert!(matches!(
        items[2],
        ModelReplayItem::FunctionCallOutput { .. }
    ));
    assert_eq!(
        items[3],
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "완료".to_owned(),
            refusal: None,
        }
    );
}

// agent-owned absolute tool budget은 binding identity가 아니라 runtime config에서 한 attempt의
// ToolExecutionRequest로 전달되고, 기본값 없음과 구분되는 exact Duration을 보존합니다.
#[test]
fn native_backend_forwards_the_optional_absolute_tool_deadline() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
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
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("r2"),
        ],
    ];
    let absolute_timeout = Duration::from_secs(37);
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(DeadlineHost {
                observed: Arc::clone(&observed),
            }),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig {
            absolute_tool_execution_timeout: Some(absolute_timeout),
            ..NativeModelBackendConfig::default()
        },
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

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::ResumableTurnFinished { .. }
    ));
    assert_eq!(&*observed.lock().unwrap(), &[Some(absolute_timeout)]);
}

// 함수 호출 완료 event가 뒤집혀 도착해도 output index 순서로 한 번씩 실행하고,
// 다음 모델 요청에도 call과 result를 같은 안정 순서로 넣는다.
#[test]
fn native_backend_executes_multiple_tools_in_model_output_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-0".to_owned(),
                call_id: "call-0".to_owned(),
                name: "read_file".to_owned(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 1,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 1,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"one"}"#.to_owned(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 0,
                item_id: "item-0".to_owned(),
                call_id: "call-0".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"zero"}"#.to_owned(),
            },
            completed("r1"),
        ],
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r2".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("r2"),
        ],
    ];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(OrderedHost {
                starts: Arc::clone(&starts),
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

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::ResumableTurnFinished { .. }
    ));
    assert_eq!(&*starts.lock().unwrap(), &["call-0", "call-1"]);
    let requests = requests.lock().unwrap();
    let replay = requests[1].input();
    assert!(matches!(
        &replay[2..],
        [
            ModelConnectorInputItem::FunctionCall { call_id: first_call, .. },
            ModelConnectorInputItem::FunctionCall { call_id: second_call, .. },
            ModelConnectorInputItem::FunctionCallOutput { call_id: first_output, .. },
            ModelConnectorInputItem::FunctionCallOutput { call_id: second_output, .. },
        ] if first_call == "call-0"
            && second_call == "call-1"
            && first_output == "call-0"
            && second_output == "call-1"
    ));
}

// 스키마가 맞지 않는 함수 호출은 실행 호스트에 도달하지 않고 실패한 Turn으로 봉인된다.
#[test]
fn native_backend_never_dispatches_invalid_tool_arguments() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![vec![
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
                arguments: "{}".to_owned(),
            },
        ]],
        ToolApprovalRequirement::Automatic,
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

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
    assert_eq!(*starts.lock().unwrap(), 0);
}

// text Activity가 열린 뒤 잘못된 tool call이 도착해도 backend가 열린 Activity를 먼저 실패로
// 봉인하므로 Runtime state machine이 terminal event를 거절하지 않는다.
#[test]
fn native_backend_failure_sequence_is_accepted_by_the_runtime() {
    let starts = Arc::new(Mutex::new(0));
    let backend = backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "partial".to_owned(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 1,
                item_id: "call-item".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 1,
                item_id: "call-item".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            },
        ]],
        ToolApprovalRequirement::Automatic,
        Arc::clone(&starts),
    );
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("요청"),
            },
            SubmissionId::new().unwrap(),
        )
        .unwrap();
    let mut finished_activities = 0;
    for _ in 0..100 {
        match runtime.poll_event().unwrap() {
            RuntimePoll::Event(AgentEvent::ActivityFinished { .. }) => finished_activities += 1,
            RuntimePoll::Event(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Failed(_),
                ..
            }) => break,
            RuntimePoll::Event(_) | RuntimePoll::Pending => {},
            RuntimePoll::Closed => panic!("runtime closed before sealing the failed Turn"),
        }
    }
    assert_eq!(finished_activities, 2);
    assert_eq!(*starts.lock().unwrap(), 0);
    assert_eq!(runtime.active_turn(), None);
}
