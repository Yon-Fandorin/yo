use std::{collections::VecDeque, num::NonZeroU64, sync::Arc};

use serde_json::json;

use super::*;
use crate::{
    AccountId, AgentEvent, AgentRuntime, ApiProtocol, ConnectorId, ModelId, NormalizedEndpoint,
    ProviderId, RuntimePoll, SubmissionId, ToolDefinition, ToolEffect, ToolExecutionError,
    ToolExecutionResult, ToolId, ToolRegistry, UserInput, fixture_session,
};

struct MockConnector {
    rounds: Arc<Mutex<VecDeque<VecDeque<ResponsesEvent>>>>,
    requests: Arc<Mutex<Vec<ResponsesRequest>>>,
}

impl ResponseConnector for MockConnector {
    fn request_url(&self) -> &str {
        "https://example.invalid/v1/responses"
    }

    fn start(
        &self,
        request: ResponsesRequest,
        _cancellation: ResponsesCancellation,
    ) -> Result<Box<dyn ResponseStream>, crate::ConnectorError> {
        self.requests.lock().unwrap().push(request);
        let events = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test declared every model round");
        Ok(Box::new(MockStream { events }))
    }
}

struct MockStream {
    events: VecDeque<ResponsesEvent>,
}

impl ResponseStream for MockStream {
    fn poll(&mut self) -> Result<ResponsesPoll, crate::ConnectorError> {
        Ok(self
            .events
            .pop_front()
            .map_or(ResponsesPoll::Closed, ResponsesPoll::Event))
    }

    fn cancel(&self) {}

    fn shutdown(&mut self) -> Result<(), crate::ConnectorError> {
        Ok(())
    }
}

#[derive(Default)]
struct MockHost {
    starts: Arc<Mutex<usize>>,
}

impl ToolExecutionHost for MockHost {
    fn identity(&self) -> &str {
        "test-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        *self.starts.lock().unwrap() += 1;
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

struct FailingStartHost;

impl ToolExecutionHost for FailingStartHost {
    fn identity(&self) -> &str {
        "failing-start-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        Err(ToolExecutionError::new("execution-host-secret"))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

struct MockExecution {
    result: Option<ToolExecutionResult>,
}

struct OrderedHost {
    starts: Arc<Mutex<Vec<String>>>,
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

impl ToolExecution for MockExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        Ok(ToolExecutionPoll::Ready)
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.result.take()
    }

    fn cancel(&self) {}

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

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

fn binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("qwen3.8-max").unwrap(),
        ConnectorId::new(ConnectorId::OPENAI_RESPONSES).unwrap(),
        ApiProtocol::OpenAiResponses,
        NormalizedEndpoint::parse("https://example.invalid/v1").unwrap(),
    )
}

fn registry(approval: ToolApprovalRequirement) -> FrozenToolRegistry {
    ToolRegistry::new([ToolDefinition::new(
        ToolId::new("read-file").unwrap(),
        "read_file",
        "reads a UTF-8 file",
        "v1",
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": false
        }),
        ToolEffect::ReadOnly,
        approval,
    )
    .unwrap()])
    .unwrap()
    .freeze()
}

struct ExactAdmission;

impl ToolSemanticAdmission for ExactAdmission {
    fn admit_arguments(
        &self,
        _definition: &crate::ToolDefinition,
        validated_argument_bytes: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Ok(validated_argument_bytes.to_owned())
    }

    fn admit_output(
        &self,
        _definition: &crate::ToolDefinition,
        bounded_output: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Ok(bounded_output.to_owned())
    }
}

struct RedactingAdmission;

impl ToolSemanticAdmission for RedactingAdmission {
    fn admit_arguments(
        &self,
        _definition: &crate::ToolDefinition,
        _validated_argument_bytes: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Ok(r#"{"path":"[redacted]"}"#.to_owned())
    }

    fn admit_output(
        &self,
        _definition: &crate::ToolDefinition,
        _bounded_output: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Ok("[redacted-output]".to_owned())
    }
}

struct RejectingAdmission;

impl ToolSemanticAdmission for RejectingAdmission {
    fn admit_arguments(
        &self,
        _definition: &crate::ToolDefinition,
        _validated_argument_bytes: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Err(crate::ToolSemanticAdmissionError::new(
            "host diagnostic contains secret.txt",
        ))
    }

    fn admit_output(
        &self,
        _definition: &crate::ToolDefinition,
        _bounded_output: &str,
    ) -> Result<String, crate::ToolSemanticAdmissionError> {
        Err(crate::ToolSemanticAdmissionError::new(
            "host diagnostic contains output-secret",
        ))
    }
}

fn context_profile() -> crate::ModelContextProfile {
    crate::ModelContextProfile::new(1_000_000, 4_096, "test-tokenizer/v1").unwrap()
}

struct FixedTokenCounter(u64);

impl crate::ModelTokenCounter for FixedTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        _request: &serde_json::Value,
    ) -> Result<u64, crate::ModelTokenCounterError> {
        Ok(self.0)
    }
}

struct FailingTokenCounter;

impl crate::ModelTokenCounter for FailingTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        _request: &serde_json::Value,
    ) -> Result<u64, crate::ModelTokenCounterError> {
        Err(crate::ModelTokenCounterError::new(
            "counter diagnostic contains request-secret",
        ))
    }
}

fn event_rounds(
    rounds: Vec<Vec<ResponsesEvent>>,
) -> Arc<Mutex<VecDeque<VecDeque<ResponsesEvent>>>> {
    Arc::new(Mutex::new(
        rounds
            .into_iter()
            .map(VecDeque::from)
            .collect::<VecDeque<_>>(),
    ))
}

fn backend(
    rounds: Vec<Vec<ResponsesEvent>>,
    approval: ToolApprovalRequirement,
    starts: Arc<Mutex<usize>>,
) -> NativeModelBackend {
    NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(approval),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost { starts }),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap()
}

fn turn() -> TurnRef {
    TurnRef::new(
        fixture_session(44),
        crate::TurnId::new(NonZeroU64::new(1).unwrap()),
    )
}

fn completed(response_id: &str) -> ResponsesEvent {
    ResponsesEvent::Terminal {
        response_id: response_id.to_owned(),
        status: ResponseTerminal::Completed,
        usage: crate::ResponsesUsage::default(),
    }
}

fn drain_until_turn(backend: &mut NativeModelBackend) -> BackendEvent {
    for _ in 0..100 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(
                event @ (BackendEvent::TurnFinished { .. }
                | BackendEvent::ResumableTurnFinished { .. }),
            ) => return event,
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before finishing the Turn"),
        }
    }
    panic!("backend did not finish within the deterministic poll budget")
}

// 모델 함수 호출은 검증·단일 실행·결과 기록을 거친 뒤 다음 모델 라운드로 이어진다.
#[test]
fn native_backend_runs_automatic_tool_once_and_replays_it_before_the_next_round() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![
            vec![
                ResponsesEvent::ResponseCreated {
                    response_id: "r1".to_owned(),
                },
                ResponsesEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                },
                ResponsesEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"README.md"}"#.to_owned(),
                },
                completed("r1"),
            ],
            vec![
                ResponsesEvent::ResponseCreated {
                    response_id: "r2".to_owned(),
                },
                ResponsesEvent::TextDelta {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                    content_index: 0,
                    delta: "완".to_owned(),
                },
                ResponsesEvent::TextDelta {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                    content_index: 0,
                    delta: "료".to_owned(),
                },
                ResponsesEvent::MessageDone {
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
                == ContinuationStrategy::ExactReplay { executor: ReplayExecutor::LocalClient }
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
        }
    );
}

// 함수 호출 완료 event가 뒤집혀 도착해도 output index 순서로 한 번씩 실행하고,
// 다음 모델 요청에도 call과 result를 같은 안정 순서로 넣는다.
#[test]
fn native_backend_executes_multiple_tools_in_model_output_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
        vec![
            ResponsesEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ResponsesEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-0".to_owned(),
                call_id: "call-0".to_owned(),
                name: "read_file".to_owned(),
            },
            ResponsesEvent::FunctionCallStarted {
                output_index: 1,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ResponsesEvent::FunctionCallDone {
                output_index: 1,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"one"}"#.to_owned(),
            },
            ResponsesEvent::FunctionCallDone {
                output_index: 0,
                item_id: "item-0".to_owned(),
                call_id: "call-0".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"zero"}"#.to_owned(),
            },
            completed("r1"),
        ],
        vec![
            ResponsesEvent::ResponseCreated {
                response_id: "r2".to_owned(),
            },
            ResponsesEvent::MessageDone {
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
            ResponsesInputItem::FunctionCall { call_id: first_call, .. },
            ResponsesInputItem::FunctionCall { call_id: second_call, .. },
            ResponsesInputItem::FunctionCallOutput { call_id: first_output, .. },
            ResponsesInputItem::FunctionCallOutput { call_id: second_output, .. },
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
            ResponsesEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ResponsesEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ResponsesEvent::FunctionCallDone {
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
            ResponsesEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ResponsesEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "partial".to_owned(),
            },
            ResponsesEvent::FunctionCallStarted {
                output_index: 1,
                item_id: "call-item".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ResponsesEvent::FunctionCallDone {
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

// 빈 assistant message item은 의도적인 최종 응답으로 보존하지만 message item이 전혀 없는
// reasoning-only 완료는 재개 가능한 Turn으로 잘못 봉인하지 않는다.
#[test]
fn native_backend_requires_a_final_assistant_message_item() {
    let starts = Arc::new(Mutex::new(0));
    let mut empty_message = backend(
        vec![vec![
            ResponsesEvent::ResponseCreated {
                response_id: "empty".to_owned(),
            },
            ResponsesEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("empty"),
        ]],
        ToolApprovalRequirement::Automatic,
        Arc::clone(&starts),
    );
    empty_message
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    empty_message
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();
    assert!(matches!(
        drain_until_turn(&mut empty_message),
        BackendEvent::ResumableTurnFinished { .. }
    ));

    let mut reasoning_only = backend(
        vec![vec![
            ResponsesEvent::ResponseCreated {
                response_id: "reasoning".to_owned(),
            },
            ResponsesEvent::ReasoningDelta {
                output_index: 0,
                item_id: "reason".to_owned(),
                channel: ReasoningChannel::Summary,
                part_index: 0,
                delta: "summary".to_owned(),
            },
            completed("reasoning"),
        ]],
        ToolApprovalRequirement::Automatic,
        starts,
    );
    reasoning_only
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    reasoning_only
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("요청"),
        })
        .unwrap();
    assert!(matches!(
        drain_until_turn(&mut reasoning_only),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        }
    ));
}

// 승인이 필요한 도구는 정확한 요청 응답 전에는 실행되지 않고 승인 후 한 번만 실행된다.
#[test]
fn native_backend_required_approval_gates_tool_execution() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![
            vec![
                ResponsesEvent::ResponseCreated {
                    response_id: "r1".to_owned(),
                },
                ResponsesEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                },
                ResponsesEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"README.md"}"#.to_owned(),
                },
                completed("r1"),
            ],
            vec![
                ResponsesEvent::ResponseCreated {
                    response_id: "r2".to_owned(),
                },
                ResponsesEvent::TextDelta {
                    output_index: 0,
                    item_id: "item-2".to_owned(),
                    content_index: 0,
                    delta: "ok".to_owned(),
                },
                ResponsesEvent::MessageDone {
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
        ResponsesEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ResponsesEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ResponsesEvent::FunctionCallDone {
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
        ResponsesEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ResponsesEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ResponsesEvent::FunctionCallDone {
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

// host가 이미 잘랐다고 보고하거나 UTF-8 경계에서 다시 잘라도 truncation marker를 포함한
// 최종 model-visible output 자체가 설정한 byte limit을 넘지 않는다.
#[test]
fn bounded_tool_output_includes_its_marker_inside_the_limit() {
    let bounded = bounded_output("가나다라마바사", 32, true);
    assert!(bounded.len() <= 32);
    assert!(bounded.ends_with("[yo: tool output truncated]"));

    let tiny = bounded_output("가나다", 5, false);
    assert!(tiny.len() <= 5);
    assert!(tiny.is_char_boundary(tiny.len()));

    let invalid = NativeModelBackendConfig {
        maximum_tool_output_bytes: TOOL_TRUNCATION_MARKER.len() - 1,
        ..NativeModelBackendConfig::default()
    };
    assert!(
        NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(Vec::new()),
                requests: Arc::new(Mutex::new(Vec::new())),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(1)),
            ),
            context_profile(),
            invalid,
        )
        .is_err()
    );
}

// local tool을 노출하면 원시 argument와 output이 semantic 경계를 우회하지 못하도록 startup을
// 거부하는지 검증합니다.
#[test]
fn native_backend_refuses_to_expose_tools_without_semantic_admission() {
    let result = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            None,
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    );

    let Err(error) = result else {
        panic!("a non-empty registry without semantic admission must fail")
    };
    assert!(error.message().contains("semantic-admission"));
}

// 예약 출력량을 제외한 input budget을 넘으면 remote call 없이 실패하고 binding을 소진시키는지
// 검증합니다.
#[test]
fn context_exhaustion_finishes_non_resumably_and_latches_the_binding() {
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(91)),
        ),
        crate::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("too much context"),
            })
            .unwrap(),
        BackendCommandEvidence::None
    ));
    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Completed,
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("context exhaustion must finish without a resumable outcome")
    };

    let next_turn = TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    let error = backend
        .execute_command(AgentCommand::StartTurn {
            turn: next_turn,
            input: UserInput::from("retry"),
        })
        .unwrap_err();
    assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
}

// 완료 응답을 replay에 더하는 순간 누적 한도를 넘더라도 실패 기록이나 재개 Anchor를
// 만들지 않고 현재 Turn을 완결한 뒤 같은 binding의 추가 호출을 차단한다.
#[test]
fn replay_exhaustion_finishes_non_resumably_and_latches_the_binding() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![vec![
            ResponsesEvent::ResponseCreated {
                response_id: "full".to_owned(),
            },
            ResponsesEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "answer".to_owned(),
            },
            ResponsesEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("full"),
        ]],
        ToolApprovalRequirement::Automatic,
        starts,
    );
    backend
        .replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            (0..4_096)
                .map(|_| ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: String::new(),
                })
                .collect(),
        ))
        .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("one item too many"),
        })
        .unwrap();

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));

    let next_turn = TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    let error = backend
        .execute_command(AgentCommand::StartTurn {
            turn: next_turn,
            input: UserInput::from("retry"),
        })
        .unwrap_err();
    assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
}

// host가 admission한 값만 Activity·replay·다음 request에 남고 원시 tool 값은 노출되지 않는지
// 검증합니다.
#[test]
fn semantic_admission_replaces_tool_values_before_activity_replay_and_next_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
        vec![
            ResponsesEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ResponsesEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ResponsesEvent::FunctionCallDone {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"secret.txt"}"#.to_owned(),
            },
            completed("r1"),
        ],
        vec![
            ResponsesEvent::ResponseCreated {
                response_id: "r2".to_owned(),
            },
            ResponsesEvent::MessageDone {
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
            Some(Box::new(RedactingAdmission)),
            Box::new(MockHost::default()),
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
            input: UserInput::from("read secret"),
        })
        .unwrap();

    let mut visible_updates = Vec::new();
    let evidence = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: crate::ActivityUpdate::TextSnapshot(text),
                ..
            }) => visible_updates.push(text),
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { evidence, .. }) => {
                break evidence;
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before resumable completion"),
        }
    };
    let visible = visible_updates.join("\n");
    assert!(!visible.contains("secret.txt"));
    assert!(!visible.contains("contents"));
    assert!(visible.contains("[redacted]"));
    assert!(visible.contains("[redacted-output]"));

    let replay = evidence.model_replay().unwrap();
    assert!(replay.items().iter().any(|item| matches!(
        item,
        ModelReplayItem::FunctionCall { arguments, .. }
            if arguments == r#"{"path":"[redacted]"}"#
    )));
    assert!(replay.items().iter().any(|item| matches!(
        item,
        ModelReplayItem::FunctionCallOutput { output, .. }
            if output == "[redacted-output]"
    )));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input().iter().any(|item| matches!(
        item,
        ResponsesInputItem::FunctionCall { arguments, .. }
            if arguments == r#"{"path":"[redacted]"}"#
    )));
    assert!(requests[1].input().iter().any(|item| matches!(
        item,
        ResponsesInputItem::FunctionCallOutput { output, .. }
            if output == "[redacted-output]"
    )));
}

// Schema mismatch 상세 설명에 model이 만든 property name이 들어가더라도, 안정적인 backend 소유
// 진단만 semantic event에 남는지 검증합니다.
#[test]
fn schema_validation_diagnostics_do_not_persist_argument_property_names() {
    let rounds = vec![vec![
        ResponsesEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ResponsesEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ResponsesEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"secret-property":"value"}"#.to_owned(),
        },
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
            Box::new(MockHost::default()),
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
            input: UserInput::from("request"),
        })
        .unwrap();

    let mut visible = String::new();
    loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => {
                visible.push_str(&format!("{event:?}"));
                break;
            },
            BackendPoll::Event(event) => visible.push_str(&format!("{event:?}")),
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the failed Turn"),
        }
    }

    assert!(!visible.contains("secret-property"));
    assert!(visible.contains("tool arguments do not match the admitted schema"));
    assert!(visible.contains(ToolValidationFailure::SchemaMismatch.code()));
}

// 주입한 admission·tokenizer의 실패 설명이 금지된 원문을 담아도 semantic event와 caller error에
// 노출되지 않는지 검증합니다.
#[test]
fn injected_policy_diagnostics_do_not_cross_the_semantic_boundary() {
    let rounds = vec![vec![
        ResponsesEvent::ResponseCreated {
            response_id: "r1".to_owned(),
        },
        ResponsesEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ResponsesEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"secret.txt"}"#.to_owned(),
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
            Some(Box::new(RejectingAdmission)),
            Box::new(MockHost::default()),
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
            input: UserInput::from("request"),
        })
        .unwrap();

    let mut visible = String::new();
    loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => {
                visible.push_str(&format!("{event:?}"));
                break;
            },
            BackendPoll::Event(event) => visible.push_str(&format!("{event:?}")),
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the failed Turn"),
        }
    }
    assert!(!visible.contains("secret.txt"));
    assert!(visible.contains("semantic admission was rejected"));

    let mut counter_backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FailingTokenCounter),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    counter_backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    let error = counter_backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("request-secret"),
        })
        .unwrap_err();
    assert!(!error.to_string().contains("request-secret"));
    assert!(error.to_string().contains("token counting failed"));

    let rounds = vec![
        vec![
            ResponsesEvent::ResponseCreated {
                response_id: "tool".to_owned(),
            },
            ResponsesEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
            },
            ResponsesEvent::FunctionCallDone {
                output_index: 0,
                item_id: "item-1".to_owned(),
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            completed("tool"),
        ],
        vec![
            ResponsesEvent::ResponseCreated {
                response_id: "answer".to_owned(),
            },
            ResponsesEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "done".to_owned(),
            },
            ResponsesEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("answer"),
        ],
    ];
    let mut host_backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(FailingStartHost),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    host_backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    host_backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("request"),
        })
        .unwrap();
    let mut visible = String::new();
    loop {
        match host_backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::ResumableTurnFinished { .. }) => {
                visible.push_str(&format!("{event:?}"));
                break;
            },
            BackendPoll::Event(event) => visible.push_str(&format!("{event:?}")),
            BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the completed Turn"),
        }
    }
    assert!(!visible.contains("execution-host-secret"));
    assert!(visible.contains("tool execution failed"));
}
