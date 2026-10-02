#[cfg(test)]
use std::mem;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, AgentEvent, AgentRuntime, BackendCommandEvidence, BackendEvent,
    ContinuationStrategy, ModelConnectorEvent, ModelConnectorInputItem, ModelReplayItem,
    ModelReplayRole, ReplayExecutor, RuntimePoll, SubmissionId, ToolApprovalRequirement,
    ToolExecution, ToolExecutionError, ToolExecutionHost, ToolExecutionOutcome,
    ToolExecutionRequest, ToolExecutionResult, ToolId, TurnOutcome, UserInput,
};

use super::support::{
    ExactAdmission, FixedTokenCounter, MockConnector, MockExecution, backend, binding, completed,
    context_profile, drain_until_turn, event_rounds, registry, turn,
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

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
                    replay_profile: yo_core::ReplayProfile::SemanticOnly,
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

// ToolResult의 host receipt를 먼저 보낸 뒤 성공한 start에만 admitted 인자의 무출력 실행 상태를 같은
// Activity로 보냅니다.
#[test]
fn running_tool_snapshot_follows_host_start_and_replays_only_admitted_values() {
    use serde_json::json;
    use yo_core::{
        ActivityKind, ActivityRef, ActivityUpdate, BackendPoll, ToolDefinition, ToolOutput,
        ToolSemanticAdmission, ToolSemanticAdmissionError, admit_standard_complete_binding,
    };

    struct Redacted;
    impl ToolSemanticAdmission for Redacted {
        fn admit_arguments(
            &self,
            _: &ToolDefinition,
            _: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            Ok(r#"{"path":"admitted.txt"}"#.to_owned())
        }

        fn admit_output(
            &self,
            _: &ToolDefinition,
            _: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            Ok("admitted-result".to_owned())
        }
    }

    struct PendingOnce {
        polls: usize,
        result: Option<ToolExecutionResult>,
    }
    impl ToolExecution for PendingOnce {
        fn poll(&mut self) -> Result<yo_core::ToolExecutionPoll, ToolExecutionError> {
            self.polls += 1;
            Ok(if self.polls == 1 {
                yo_core::ToolExecutionPoll::Pending
            } else {
                yo_core::ToolExecutionPoll::Ready
            })
        }

        fn take_result(&mut self) -> Option<ToolExecutionResult> {
            self.result.take()
        }

        fn cancel(&self) {}

        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }

    struct PendingOnceHost(Arc<Mutex<usize>>);
    impl ToolExecutionHost for PendingOnceHost {
        fn identity(&self) -> &str {
            "quiet-running-host-v1"
        }

        fn is_available(&self, _: &ToolId) -> bool {
            true
        }

        fn start(
            &mut self,
            _: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            *self.0.lock().unwrap() += 1;
            Ok(Box::new(PendingOnce {
                polls: 0,
                result: Some(ToolExecutionResult::new(
                    ToolExecutionOutcome::Completed,
                    "host-only result",
                    false,
                )),
            }))
        }

        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }

    let starts = Arc::new(Mutex::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "tool".to_owned(),
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
                arguments: r#"{"path":"RAW_PATH_SECRET"}"#.to_owned(),
            },
            completed("tool"),
        ],
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "answer".to_owned(),
            },
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "done".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("answer"),
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
            Box::new(admit_standard_complete_binding),
            Some(Box::new(Redacted)),
            Box::new(PendingOnceHost(Arc::clone(&starts))),
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
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let mut call_activity = None;
    let mut result_activity: Option<ActivityRef> = None;
    let mut saw_receipt = false;
    let mut saw_running = false;
    let mut saw_pending_poll = false;
    let mut saw_terminal_result = false;
    let mut terminal = false;
    let mut visible = String::new();
    for _ in 0..150 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(event) => {
                match &event {
                    BackendEvent::ActivityStarted { activity, kind } => match kind {
                        ActivityKind::ToolCall => call_activity = Some(*activity),
                        ActivityKind::ToolResult => {
                            assert_ne!(call_activity, Some(*activity));
                            assert_eq!(*starts.lock().unwrap(), 0);
                            result_activity = Some(*activity);
                        },
                        _ => {},
                    },
                    BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(text),
                    } if result_activity == Some(*activity) => {
                        match ToolOutput::from_snapshot(text) {
                            Some(output) => match output.result.as_ref() {
                                None => {
                                    assert_eq!(*starts.lock().unwrap(), 1);
                                    assert_eq!(output.tool, "read_file");
                                    assert_eq!(
                                        output.arguments,
                                        Some(json!({"path":"admitted.txt"}))
                                    );
                                    assert!(output.plain_text.contains("admitted.txt"));
                                    assert!(!text.contains("RAW_PATH_SECRET"));
                                    assert!(!output.plain_text.contains("RAW_PATH_SECRET"));
                                    saw_running = true;
                                },
                                Some(result) => {
                                    assert_eq!(result["content"][0]["text"], "admitted-result");
                                    assert_eq!(
                                        output.arguments,
                                        Some(json!({"path":"admitted.txt"}))
                                    );
                                    saw_terminal_result = true;
                                },
                            },
                            None => {
                                let receipt: serde_json::Value =
                                    serde_json::from_str(text).unwrap();
                                assert_eq!(*starts.lock().unwrap(), 0);
                                assert_eq!(receipt["call_id"], "call-1");
                                assert_eq!(receipt["tool_id"], "read-file");
                                assert_eq!(receipt["execution_host"], "quiet-running-host-v1");
                                assert_eq!(receipt["attempt"], 1);
                                saw_receipt = true;
                            },
                        }
                    },
                    BackendEvent::ResumableTurnFinished { .. } => {
                        visible.push_str(&format!("{event:?}"));
                        terminal = true;
                        break;
                    },
                    _ => {},
                }
                visible.push_str(&format!("{event:?}"));
            },
            BackendPoll::Pending => {
                if saw_running && !saw_pending_poll {
                    saw_pending_poll = true;
                }
            },
            BackendPoll::Closed => panic!("backend closed before the completed Turn"),
        }
    }
    assert!(terminal);
    assert!(saw_receipt && saw_running && saw_pending_poll && saw_terminal_result);
    assert_eq!(*starts.lock().unwrap(), 1);
    assert!(!visible.contains("RAW_PATH_SECRET"));
    assert!(!visible.contains("host-only result"));
    let result_activity = result_activity.unwrap();
    assert_ne!(call_activity, Some(result_activity));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input().iter().any(|item| matches!(item,
        ModelConnectorInputItem::FunctionCallOutput { call_id, output }
            if call_id == "call-1" && output == "admitted-result"
    )));
}

// admitted 인자만으로 만든 ToolOutput이 profile 상한을 넘으면 active executor와 ToolResult
// receipt를 유지합니다.
#[test]
fn oversized_running_tool_profile_does_not_fail_a_started_execution() {
    use yo_core::{ActivityKind, ActivityUpdate, BackendPoll, ToolOutput};

    struct PendingExecution {
        cancellations: Arc<Mutex<usize>>,
        shutdowns: Arc<Mutex<usize>>,
    }
    impl ToolExecution for PendingExecution {
        fn poll(&mut self) -> Result<yo_core::ToolExecutionPoll, ToolExecutionError> {
            Ok(yo_core::ToolExecutionPoll::Pending)
        }

        fn take_result(&mut self) -> Option<ToolExecutionResult> {
            None
        }

        fn cancel(&self) {
            *self.cancellations.lock().unwrap() += 1;
        }

        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            *self.shutdowns.lock().unwrap() += 1;
            Ok(())
        }
    }

    struct PendingHost {
        starts: Arc<Mutex<usize>>,
        cancellations: Arc<Mutex<usize>>,
        shutdowns: Arc<Mutex<usize>>,
    }
    impl ToolExecutionHost for PendingHost {
        fn identity(&self) -> &str {
            "oversized-profile-host-v1"
        }

        fn is_available(&self, _: &ToolId) -> bool {
            true
        }

        fn start(
            &mut self,
            _: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            *self.starts.lock().unwrap() += 1;
            Ok(Box::new(PendingExecution {
                cancellations: Arc::clone(&self.cancellations),
                shutdowns: Arc::clone(&self.shutdowns),
            }))
        }

        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }

    let path = "x".repeat(9 * 1024 * 1024);
    let arguments = format!("{{\"path\":\"{path}\"}}");
    let maximum_argument_bytes = 10 * 1024 * 1024;
    assert!(arguments.len() < maximum_argument_bytes);
    assert!(path.len() * 2 > ToolOutput::MAX_SNAPSHOT_BYTES);

    let starts = Arc::new(Mutex::new(0));
    let cancellations = Arc::new(Mutex::new(0));
    let shutdowns = Arc::new(Mutex::new(0));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "tool".to_owned(),
                },
                ModelConnectorEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-large".to_owned(),
                    name: "read_file".to_owned(),
                },
                ModelConnectorEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "item-1".to_owned(),
                    call_id: "call-large".to_owned(),
                    name: "read_file".to_owned(),
                    arguments,
                },
                completed("tool"),
            ]]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(PendingHost {
                starts: Arc::clone(&starts),
                cancellations: Arc::clone(&cancellations),
                shutdowns: Arc::clone(&shutdowns),
            }),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig {
            maximum_tool_argument_bytes: maximum_argument_bytes,
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
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let mut result_activity = None;
    let mut saw_receipt = false;
    let mut saw_running_profile = false;
    let mut started_without_profile = false;
    for _ in 0..100 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ToolResult,
            }) => result_activity = Some(activity),
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) if result_activity == Some(activity) => {
                if let Some(output) = ToolOutput::from_snapshot(&text) {
                    saw_running_profile |= output.result.is_none();
                } else {
                    let receipt: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(receipt["call_id"], "call-large");
                    assert_eq!(receipt["execution_host"], "oversized-profile-host-v1");
                    saw_receipt = true;
                }
            },
            BackendPoll::Pending if saw_receipt => {
                started_without_profile = *starts.lock().unwrap() == 1
                    && backend
                        .turn
                        .as_ref()
                        .is_some_and(|state| state.active_tool.is_some());
                if started_without_profile {
                    break;
                }
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before the pending execution"),
        }
    }
    assert!(saw_receipt);
    assert!(!saw_running_profile);
    assert!(started_without_profile);
    assert_eq!(*starts.lock().unwrap(), 1);

    backend
        .execute_command(AgentCommand::InterruptTurn { turn: turn() })
        .unwrap();
    let interrupted = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(
                event @ BackendEvent::TurnFinished {
                    outcome: TurnOutcome::Interrupted,
                    ..
                },
            ) => Some(event),
            BackendPoll::Event(_) | BackendPoll::Pending => None,
            other => panic!("started executor did not remain interruptible: {other:?}"),
        })
        .expect("started executor did not report interruption within 100 polls");
    assert!(matches!(interrupted, BackendEvent::TurnFinished { .. }));
    assert_eq!(*starts.lock().unwrap(), 1);
    assert_eq!(*cancellations.lock().unwrap(), 1);
    assert_eq!(*shutdowns.lock().unwrap(), 1);
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
            Box::new(yo_core::admit_standard_complete_binding),
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
            Box::new(yo_core::admit_standard_complete_binding),
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

// 성공·실패·중단 결과의 문자열은 구조화 표시와 다음 모델 요청에 동일하게 전달하며 JSON을 미디어로
// 해석하지 않는다. 표시 확장이 상한을 넘으면 원문 receipt로 유지한다.
#[test]
fn managed_tool_output_preserves_literal_results_and_outcomes() {
    use yo_core::{ActivityOutcome, ActivityUpdate, BackendPoll, ToolOutput};

    const LITERAL: &str =
        "fn main() {}\n```\n{\"content\":[{\"type\":\"image\",\"data\":\"literal\"}]}";
    struct LiteralHost(ToolExecutionOutcome, bool, bool);
    impl ToolExecutionHost for LiteralHost {
        fn identity(&self) -> &str {
            "literal-host-v1"
        }
        fn is_available(&self, _: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            _: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            Ok(Box::new(MockExecution {
                result: Some(ToolExecutionResult::new(
                    self.0,
                    if self.1 {
                        "\0".repeat(2 * 1024 * 1024)
                    } else {
                        LITERAL.to_owned()
                    },
                    self.2,
                )),
            }))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }

    for (outcome, oversized_presentation, host_truncated, output_limit) in [
        (ToolExecutionOutcome::Completed, false, false, None),
        (ToolExecutionOutcome::Failed, false, false, None),
        (ToolExecutionOutcome::Interrupted, false, false, None),
        (ToolExecutionOutcome::Completed, true, false, None),
        (ToolExecutionOutcome::Completed, false, true, None),
        (
            ToolExecutionOutcome::Completed,
            false,
            false,
            Some(LITERAL.len()),
        ),
        (
            ToolExecutionOutcome::Completed,
            false,
            false,
            Some(LITERAL.len() - 1),
        ),
    ] {
        let requests = Arc::new(Mutex::new(Vec::new()));
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
                    arguments: r#"{"path":"src/main.rs"}"#.to_owned(),
                },
                completed("r1"),
            ],
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r2".to_owned(),
                },
                ModelConnectorEvent::MessageDone {
                    output_index: 0,
                    item_id: "answer".to_owned(),
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
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(LiteralHost(outcome, oversized_presentation, host_truncated)),
                Box::new(FixedTokenCounter(1)),
            ),
            context_profile(),
            NativeModelBackendConfig {
                maximum_tool_output_bytes: output_limit.unwrap_or(4 * 1024 * 1024),
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
                input: UserInput::from("read"),
            })
            .unwrap();
        let mut observed = None;
        let mut legacy = None;
        let mut finished = None;
        let mut terminal = false;
        for _ in 0..100 {
            match backend.poll_event().unwrap() {
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(text),
                }) => {
                    if let Some(output) =
                        ToolOutput::from_snapshot(&text).filter(|output| output.result.is_some())
                    {
                        observed = Some((activity, output));
                    } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                        && value.get("output").is_some_and(|output| output.is_string())
                    {
                        legacy = Some(value);
                    }
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome })
                    if observed.as_ref().is_some_and(|(id, _)| *id == activity) =>
                {
                    finished = Some(outcome)
                },
                BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                    terminal = true;
                    break;
                },
                _ => {},
            }
        }
        assert!(terminal);
        if oversized_presentation {
            assert!(observed.is_none());
            let legacy = legacy.expect("oversized presentation retains the legacy receipt");
            assert_eq!(legacy["call_id"], "call-1");
            let original = legacy["output"].as_str().unwrap();
            assert_eq!(original.len(), 2 * 1024 * 1024);
            assert!(original.bytes().all(|byte| byte == 0));
            let requests = requests.lock().unwrap();
            assert!(requests[1].input().iter().any(|item| matches!(item,
                ModelConnectorInputItem::FunctionCallOutput { call_id, output } if call_id == "call-1" && output == original
            )));
            continue;
        }
        assert!(legacy.is_none());
        let (_, output) = observed.unwrap();
        assert_eq!(output.tool, "read_file");
        assert_eq!(output.arguments.unwrap()["path"], "src/main.rs");
        let result = output.result.unwrap();
        assert_eq!(result["execution_host"], "literal-host-v1");
        assert_eq!(result["tool_id"], "read-file");
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(
            result["isError"],
            outcome != ToolExecutionOutcome::Completed
        );
        let literal = result["content"][0]["text"].as_str().unwrap();
        let locally_truncated = output_limit.is_some_and(|limit| LITERAL.len() > limit);
        assert_eq!(result["truncated"], host_truncated || locally_truncated);
        if let Some(limit) = output_limit.filter(|_| locally_truncated) {
            assert_eq!(literal.len(), limit);
            assert!(literal.ends_with("[yo: tool output truncated]"));
        } else {
            assert!(literal.contains("\"type\":\"image\""));
        }
        if host_truncated {
            assert!(literal.ends_with("[yo: tool output truncated]"));
        }
        assert!(output.plain_text.contains(literal));
        let expected_status = match outcome {
            ToolExecutionOutcome::Completed => "completed",
            ToolExecutionOutcome::Failed => "failed",
            ToolExecutionOutcome::Interrupted => "interrupted",
        };
        assert_eq!(result["outcome"], expected_status);
        match (outcome, finished.unwrap()) {
            (ToolExecutionOutcome::Completed, ActivityOutcome::Completed)
            | (ToolExecutionOutcome::Interrupted, ActivityOutcome::Interrupted) => {},
            (ToolExecutionOutcome::Failed, ActivityOutcome::Failed(failure)) => {
                assert_eq!(failure.message(), "tool execution failed")
            },
            other => panic!("changed tool outcome: {other:?}"),
        }
        let requests = requests.lock().unwrap();
        assert!(requests[1].input().iter().any(|item| matches!(item,
            ModelConnectorInputItem::FunctionCallOutput { call_id, output } if call_id == "call-1" && output == literal
        )));
    }
}

// 진행 snapshot은 별도 승인 뒤에만 게시되고 replay에는 최종 결과만 들어가며 실패 시 실행을
// 정리한다.
#[test]
fn progress_requires_admission_and_never_enters_model_replay() {
    use serde_json::json;
    use yo_core::{
        ActivityUpdate, BackendPoll, ToolDefinition, ToolExecutionPoll, ToolExecutionProgress,
        ToolOutput, ToolSemanticAdmission, ToolSemanticAdmissionError,
        admit_standard_complete_binding,
    };
    struct Admission(u8);
    impl ToolSemanticAdmission for Admission {
        fn admit_arguments(
            &self,
            _: &ToolDefinition,
            _: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            Ok(r#"{"path":"admitted-file"}"#.to_owned())
        }
        fn admit_output(
            &self,
            _: &ToolDefinition,
            _: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            Ok("admitted final".to_owned())
        }
        fn admit_progress(
            &self,
            _: &ToolDefinition,
            _: &str,
        ) -> Result<Option<String>, ToolSemanticAdmissionError> {
            match self.0 {
                1 => Err(ToolSemanticAdmissionError::new("raw diagnostic secret")),
                2 => Ok(Some("x".repeat(65))),
                7 => Ok(Some("a".repeat(64))),
                _ => Ok(Some("admitted progress".to_owned())),
            }
        }
    }
    struct Execution {
        polls: u8,
        sent: bool,
        mode: u8,
        shutdowns: Arc<Mutex<usize>>,
    }
    impl ToolExecution for Execution {
        fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
            self.polls += 1;
            Ok(if self.polls < 3 {
                ToolExecutionPoll::Pending
            } else {
                ToolExecutionPoll::Ready
            })
        }
        fn take_progress(&mut self) -> Option<ToolExecutionProgress> {
            if mem::replace(&mut self.sent, true) {
                None
            } else {
                Some(ToolExecutionProgress {
                    output: if self.mode == 5 {
                        "x".repeat(65)
                    } else if self.mode == 6 {
                        "x".repeat(64)
                    } else {
                        "raw progress secret".to_owned()
                    },
                    truncated: self.mode == 4,
                })
            }
        }
        fn take_result(&mut self) -> Option<ToolExecutionResult> {
            Some(ToolExecutionResult::new(
                ToolExecutionOutcome::Completed,
                "raw final",
                false,
            ))
        }
        fn cancel(&self) {}
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            *self.shutdowns.lock().unwrap() += 1;
            Ok(())
        }
    }
    struct Host(u8, Arc<Mutex<usize>>);
    impl ToolExecutionHost for Host {
        fn identity(&self) -> &str {
            "progress-host"
        }
        fn is_available(&self, _: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            _: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            Ok(Box::new(Execution {
                polls: 0,
                sent: false,
                mode: self.0,
                shutdowns: Arc::clone(&self.1),
            }))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }
    for mode in 0..8 {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdowns = Arc::new(Mutex::new(0));
        let rounds = vec![
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r1".to_owned(),
                },
                ModelConnectorEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "item".to_owned(),
                    call_id: "call".to_owned(),
                    name: "read_file".to_owned(),
                },
                ModelConnectorEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "item".to_owned(),
                    call_id: "call".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: json!({"path":"file"}).to_string(),
                },
                completed("r1"),
            ],
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r2".to_owned(),
                },
                completed("r2"),
            ],
        ];
        let admission: Box<dyn ToolSemanticAdmission> = if mode == 3 {
            Box::new(ExactAdmission)
        } else {
            Box::new(Admission(mode))
        };
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(rounds),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(admit_standard_complete_binding),
                Some(admission),
                Box::new(Host(mode, Arc::clone(&shutdowns))),
                Box::new(FixedTokenCounter(1)),
            ),
            context_profile(),
            NativeModelBackendConfig {
                maximum_tool_output_bytes: 64,
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
                input: UserInput::from("read"),
            })
            .unwrap();
        let mut profiles = Vec::new();
        let mut visible = String::new();
        let mut terminal = false;
        for _ in 0..200 {
            if let BackendPoll::Event(event) = backend.poll_event().unwrap() {
                visible.push_str(&format!("{event:?}"));
                match event {
                    BackendEvent::ActivityUpdated {
                        update: ActivityUpdate::TextSnapshot(text),
                        ..
                    } => {
                        if let Some(profile) = ToolOutput::from_snapshot(&text) {
                            profiles.push(profile);
                        }
                    },
                    BackendEvent::TurnFinished { .. }
                    | BackendEvent::ResumableTurnFinished { .. } => {
                        terminal = true;
                        break;
                    },
                    _ => {},
                }
            }
        }
        assert!(terminal, "mode {mode}");
        assert!(
            !visible.contains("raw progress secret") && !visible.contains("raw diagnostic secret")
        );
        let progress = profiles
            .iter()
            .filter(|profile| {
                profile
                    .result
                    .as_ref()
                    .and_then(|result| result.get("progress"))
                    == Some(&json!(true))
            })
            .collect::<Vec<_>>();
        assert_eq!(progress.len(), usize::from(matches!(mode, 0 | 6 | 7)));
        if matches!(mode, 0 | 6 | 7) {
            assert_eq!(progress[0].arguments, Some(json!({"path":"admitted-file"})));
            assert_eq!(
                progress[0].result.as_ref().unwrap()["content"][0]["text"],
                if mode == 7 {
                    "a".repeat(64)
                } else {
                    "admitted progress".to_owned()
                }
            );
        }
        assert_eq!(*shutdowns.lock().unwrap(), 1);
        let requests = requests.lock().unwrap();
        let serialized = format!("{requests:?}");
        assert!(
            !serialized.contains("admitted progress")
                && !serialized.contains("raw progress secret")
        );
        if mode == 1 || mode == 2 {
            assert_eq!(requests.len(), 1);
        } else {
            assert_eq!(requests.len(), 2);
            assert!(serialized.contains(if mode == 3 {
                "raw final"
            } else {
                "admitted final"
            }));
        }
    }
}

// 보존 원문은 별도 admission·상한을 통과한 뒤 표시되고 모델에는 짧은 결과만 전달된다.
// 상한 첫 초과, admission 거절·팽창, profile 인코딩 초과는 원문 유출·재실행 없이 실패한다.
#[test]
fn retained_output_is_admitted_separately_and_never_enters_model_replay() {
    use serde_json::json;
    use yo_core::{
        ActivityUpdate, BackendPoll, ToolDefinition, ToolOutput, ToolSemanticAdmission,
        ToolSemanticAdmissionError, admit_standard_complete_binding,
    };

    struct Host(ToolExecutionResult, Option<usize>);
    impl ToolExecutionHost for Host {
        fn identity(&self) -> &str {
            "retention-host"
        }
        fn is_available(&self, _: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            request: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            assert_eq!(request.maximum_output_bytes, 64);
            assert_eq!(request.maximum_retained_output_bytes, self.1);
            Ok(Box::new(MockExecution {
                result: Some(self.0.clone()),
            }))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }
    struct Admission;
    impl ToolSemanticAdmission for Admission {
        fn admit_arguments(
            &self,
            _: &ToolDefinition,
            text: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            Ok(text.into())
        }
        fn admit_output(
            &self,
            _: &ToolDefinition,
            text: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            match text {
                "unsafe retained secret" => Err(ToolSemanticAdmissionError::new("rejected")),
                "redact retained" => Ok("safe retained".into()),
                "expand retained" => Ok("x".repeat(129)),
                _ => Ok(text.into()),
            }
        }
    }
    let mut cases = Vec::new();
    for outcome in [
        ToolExecutionOutcome::Completed,
        ToolExecutionOutcome::Failed,
        ToolExecutionOutcome::Interrupted,
    ] {
        for truncated in [false, true] {
            cases.push((
                outcome,
                "retained-only".to_owned(),
                truncated,
                Some(128),
                Some("retained-only".to_owned()),
            ));
        }
    }
    for (text, limit, expected) in [
        ("x".repeat(128), Some(128), Some("x".repeat(128))),
        ("x".repeat(129), Some(128), None),
        ("unsafe retained secret".to_owned(), Some(128), None),
        (
            "redact retained".to_owned(),
            Some(128),
            Some("safe retained".to_owned()),
        ),
        ("expand retained".to_owned(), Some(128), None),
        ("retained-only".to_owned(), None, None),
        (
            "retained-only".to_owned(),
            Some(ToolOutput::MAX_SNAPSHOT_BYTES),
            Some("retained-only".to_owned()),
        ),
        (
            "\0".repeat(ToolOutput::MAX_SNAPSHOT_BYTES / 6),
            Some(8 * 1024 * 1024),
            None,
        ),
    ] {
        cases.push((
            ToolExecutionOutcome::Completed,
            text,
            false,
            limit,
            expected,
        ));
    }
    for (outcome, retained, truncated, limit, expected) in cases {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let rounds = vec![
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r1".into(),
                },
                ModelConnectorEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "item-1".into(),
                    call_id: "call-1".into(),
                    name: "read_file".into(),
                },
                ModelConnectorEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "item-1".into(),
                    call_id: "call-1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path":"file"}).to_string(),
                },
                completed("r1"),
            ],
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r2".into(),
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
                Box::new(admit_standard_complete_binding),
                Some(Box::new(Admission)),
                Box::new(Host(
                    ToolExecutionResult::new(outcome, "m".repeat(96), false)
                        .with_retained_output(retained, truncated),
                    limit,
                )),
                Box::new(FixedTokenCounter(1)),
            ),
            context_profile(),
            NativeModelBackendConfig {
                maximum_tool_output_bytes: 64,
                maximum_retained_tool_output_bytes: limit,
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
                input: UserInput::from("read"),
            })
            .unwrap();
        let mut profile = None;
        let mut visible = String::new();
        let mut terminal = false;
        for _ in 0..100 {
            if let BackendPoll::Event(event) = backend.poll_event().unwrap() {
                visible.push_str(&format!("{event:?}"));
                match event {
                    BackendEvent::ActivityUpdated {
                        update: ActivityUpdate::TextSnapshot(text),
                        ..
                    } => {
                        if let Some(output) = ToolOutput::from_snapshot(&text)
                            .filter(|output| output.result.is_some())
                        {
                            profile = Some(output);
                        }
                    },
                    BackendEvent::TurnFinished { .. }
                    | BackendEvent::ResumableTurnFinished { .. } => {
                        terminal = true;
                        break;
                    },
                    _ => {},
                }
            }
        }
        assert!(terminal);
        assert!(!visible.contains("unsafe retained secret"));
        assert!(!visible.contains("redact retained"));
        let requests = requests.lock().unwrap();
        if let Some(expected) = expected {
            let profile = profile.expect("retained profile");
            assert!(profile.plain_text.ends_with(&expected));
            let result = profile.result.unwrap();
            assert_eq!(result["retainedOutput"]["truncated"], truncated);
            assert_eq!(result["truncated"], true);
            assert_eq!(result["content"][0]["text"].as_str().unwrap().len(), 64);
            assert_eq!(requests.len(), 2);
            let replay = format!("{:?}", requests[1]);
            assert!(!replay.contains(&expected));
            assert!(replay.contains("tool output truncated"));
        } else {
            assert!(profile.is_none());
            assert_eq!(requests.len(), 1);
            assert!(visible.contains("tool retained output"));
        }
    }
}

// Native edit의 bounded capture는 호출 인자와 별도로 표시하고 완료 receipt와 정확히 상관시킨다.
#[test]
fn edit_publication_evidence_is_admitted_correlated_and_replay_neutral() {
    use serde_json::json;
    use yo_core::{
        ActivityOutcome, ActivityUpdate, BackendPoll, FilePublicationEvidence,
        FilePublicationEvidenceState, FilePublicationEvidenceUnavailableReason,
        ModelConnectorInputItem, TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition,
        ToolEffect, ToolOutput, ToolRegistry, ToolSemanticAdmission, ToolSemanticAdmissionError,
    };

    struct PublicationHost {
        result: ToolExecutionResult,
        retained_limit: Option<usize>,
    }
    impl ToolExecutionHost for PublicationHost {
        fn identity(&self) -> &str {
            "native-edit-host-v1"
        }
        fn is_available(&self, _: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            request: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            assert_eq!(request.call.definition().id().as_str(), "edit-file");
            assert_eq!(request.call.definition().wire_name(), "edit_file");
            assert_eq!(request.call.call_id(), "edit-call");
            assert_eq!(request.maximum_retained_output_bytes, self.retained_limit);
            Ok(Box::new(MockExecution {
                result: Some(self.result.clone()),
            }))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    enum EditAdmissionMode {
        Preserve,
        RedactBefore,
        RejectBefore,
        ExpandBefore,
        RedactArguments,
        PanicBefore,
    }

    struct EditAdmission {
        mode: EditAdmissionMode,
    }
    impl ToolSemanticAdmission for EditAdmission {
        fn admit_arguments(
            &self,
            _: &ToolDefinition,
            arguments: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            if matches!(self.mode, EditAdmissionMode::RedactArguments) {
                Ok(r#"{}"#.to_owned())
            } else {
                Ok(arguments.to_owned())
            }
        }
        fn admit_output(
            &self,
            _: &ToolDefinition,
            output: &str,
        ) -> Result<String, ToolSemanticAdmissionError> {
            if output != "before image\n\"\\" {
                return Ok(output.to_owned());
            }
            match self.mode {
                EditAdmissionMode::RedactBefore => Ok("redacted image".to_owned()),
                EditAdmissionMode::RejectBefore => {
                    Err(ToolSemanticAdmissionError::new("captured source rejected"))
                },
                EditAdmissionMode::ExpandBefore => Ok(format!("{output}expanded")),
                EditAdmissionMode::PanicBefore => panic!("captured content admission panicked"),
                EditAdmissionMode::Preserve | EditAdmissionMode::RedactArguments => {
                    Ok(output.to_owned())
                },
            }
        }
    }

    fn edit_registry(argument_limit: usize) -> yo_core::FrozenToolRegistry {
        ToolRegistry::new([ToolDefinition::new(
            ToolId::new("edit-file").unwrap(),
            "edit_file",
            "edits exact text in one workspace file",
            TOOL_SCHEMA_DIALECT,
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": {"type": "string"},
                                "newText": {"type": "string"}
                            },
                            "required": ["oldText", "newText"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }),
            ToolEffect::WorkspaceWrite,
            ToolApprovalRequirement::Automatic,
        )
        .unwrap()
        .with_argument_byte_limit(argument_limit)
        .unwrap()])
        .unwrap()
        .freeze()
    }

    fn run_edit(
        arguments: String,
        result: ToolExecutionResult,
        retained_limit: Option<usize>,
        admission_mode: EditAdmissionMode,
        argument_limit: usize,
    ) -> (
        Option<ToolOutput>,
        Option<ActivityOutcome>,
        Option<String>,
        usize,
        bool,
    ) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let rounds = vec![
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r1".to_owned(),
                },
                ModelConnectorEvent::FunctionCallStarted {
                    output_index: 0,
                    item_id: "edit-item".to_owned(),
                    call_id: "edit-call".to_owned(),
                    name: "edit_file".to_owned(),
                },
                ModelConnectorEvent::FunctionCallDone {
                    output_index: 0,
                    item_id: "edit-item".to_owned(),
                    call_id: "edit-call".to_owned(),
                    name: "edit_file".to_owned(),
                    arguments,
                },
                completed("r1"),
            ],
            vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "r2".to_owned(),
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
            edit_registry(argument_limit),
            NativeModelBackendServices::new(
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(EditAdmission {
                    mode: admission_mode,
                })),
                Box::new(PublicationHost {
                    result,
                    retained_limit,
                }),
                Box::new(FixedTokenCounter(1)),
            ),
            context_profile(),
            NativeModelBackendConfig {
                maximum_tool_output_bytes: 4096,
                maximum_retained_tool_output_bytes: retained_limit,
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
                input: UserInput::from("inspect edit"),
            })
            .unwrap();
        let mut observed_activity = None;
        let mut profile = None;
        let mut finished = None;
        let mut terminal = false;
        for _ in 0..200 {
            match backend.poll_event().unwrap() {
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(text),
                }) => {
                    if let Some(output) = ToolOutput::from_snapshot(&text)
                        .filter(|output| output.tool == "edit_file" && output.result.is_some())
                    {
                        observed_activity = Some(activity);
                        profile = Some(output);
                    }
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome })
                    if observed_activity == Some(activity) =>
                {
                    finished = Some(outcome);
                },
                BackendPoll::Event(BackendEvent::TurnFinished { .. })
                | BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                    terminal = true;
                    break;
                },
                _ => {},
            }
        }
        let requests = requests.lock().unwrap();
        let replay_output = requests.get(1).and_then(|request| {
            request.input().iter().find_map(|item| match item {
                ModelConnectorInputItem::FunctionCallOutput { call_id, output }
                    if call_id == "edit-call" =>
                {
                    Some(output.clone())
                },
                _ => None,
            })
        });
        (profile, finished, replay_output, requests.len(), terminal)
    }

    let native_output = r#"{"path":"src/file.txt","status":"ok","replacements":1}"#;
    let before = "before image\n\"\\";
    let after = "after image\n\"\\";
    let complete_profile = FilePublicationEvidence::complete("src/file.txt", before, after)
        .unwrap()
        .to_snapshot()
        .unwrap();
    let arguments = json!({
        "path": "src/file.txt",
        "edits": [{"oldText": "before image", "newText": "after image"}]
    })
    .to_string();
    let (complete, finished, replay, request_count, terminal) = run_edit(
        arguments.clone(),
        ToolExecutionResult::new(ToolExecutionOutcome::Completed, native_output, false)
            .with_retained_output(complete_profile.clone(), false),
        Some(4096),
        EditAdmissionMode::Preserve,
        20 * 1024 * 1024,
    );
    assert!(terminal);
    assert_eq!(request_count, 2);
    assert_eq!(finished, Some(ActivityOutcome::Completed));
    let complete = complete.expect("typed edit publication snapshot");
    assert_eq!(complete.tool, "edit_file");
    assert_eq!(complete.arguments.as_ref().unwrap()["path"], "src/file.txt");
    let result = complete.result.as_ref().unwrap();
    assert_eq!(result["call_id"], "edit-call");
    assert_eq!(result["tool_id"], "edit-file");
    assert_eq!(result["execution_host"], "native-edit-host-v1");
    assert_eq!(result["outcome"], "completed");
    assert_eq!(result["truncated"], false);
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["text"], native_output);
    assert_eq!(result["publicationEvidence"], complete_profile);
    let admitted_evidence =
        FilePublicationEvidence::from_snapshot(result["publicationEvidence"].as_str().unwrap())
            .unwrap();
    assert_eq!(admitted_evidence.path(), "src/file.txt");
    assert_eq!(
        admitted_evidence.state(),
        &FilePublicationEvidenceState::Complete {
            before: before.to_owned(),
            after: after.to_owned(),
        }
    );
    assert!(!complete.plain_text.contains(before));
    assert!(!complete.plain_text.contains(after));
    assert_eq!(replay.as_deref(), Some(native_output));

    let invalid_profile = "not a versioned evidence profile".to_owned();
    let wrong_path_profile = FilePublicationEvidence::complete("other.txt", before, after)
        .unwrap()
        .to_snapshot()
        .unwrap();
    let oversized_profile = complete_profile.clone();
    let cases = [
        (
            "disabled",
            None,
            Some((complete_profile.clone(), false)),
            EditAdmissionMode::Preserve,
            Some(FilePublicationEvidenceUnavailableReason::Disabled),
        ),
        (
            "missing",
            Some(4096),
            None,
            EditAdmissionMode::Preserve,
            Some(FilePublicationEvidenceUnavailableReason::InvalidEvidence),
        ),
        (
            "malformed",
            Some(4096),
            Some((invalid_profile, false)),
            EditAdmissionMode::Preserve,
            Some(FilePublicationEvidenceUnavailableReason::InvalidEvidence),
        ),
        (
            "path mismatch",
            Some(4096),
            Some((wrong_path_profile, false)),
            EditAdmissionMode::Preserve,
            Some(FilePublicationEvidenceUnavailableReason::InvalidEvidence),
        ),
        (
            "truncated",
            Some(4096),
            Some((complete_profile.clone(), true)),
            EditAdmissionMode::Preserve,
            Some(FilePublicationEvidenceUnavailableReason::InvalidEvidence),
        ),
        (
            "over bound",
            Some(8),
            Some((oversized_profile, false)),
            EditAdmissionMode::Preserve,
            Some(FilePublicationEvidenceUnavailableReason::OverBound),
        ),
        (
            "changed admission",
            Some(4096),
            Some((complete_profile.clone(), false)),
            EditAdmissionMode::RedactBefore,
            Some(FilePublicationEvidenceUnavailableReason::SemanticAdmission),
        ),
        (
            "rejected captured content",
            Some(4096),
            Some((complete_profile.clone(), false)),
            EditAdmissionMode::RejectBefore,
            Some(FilePublicationEvidenceUnavailableReason::SemanticAdmission),
        ),
        (
            "expanded captured content",
            Some(4096),
            Some((complete_profile.clone(), false)),
            EditAdmissionMode::ExpandBefore,
            Some(FilePublicationEvidenceUnavailableReason::SemanticAdmission),
        ),
        (
            "panicking captured-content admission",
            Some(4096),
            Some((complete_profile.clone(), false)),
            EditAdmissionMode::PanicBefore,
            Some(FilePublicationEvidenceUnavailableReason::GenerationFailed),
        ),
    ];
    for (label, retained_limit, retained, admission_mode, expected_reason) in cases {
        let result = if let Some((text, truncated)) = retained {
            ToolExecutionResult::new(ToolExecutionOutcome::Completed, native_output, false)
                .with_retained_output(text, truncated)
        } else {
            ToolExecutionResult::new(ToolExecutionOutcome::Completed, native_output, false)
        };
        let (profile, finished, replay, request_count, terminal) = run_edit(
            arguments.clone(),
            result,
            retained_limit,
            admission_mode,
            20 * 1024 * 1024,
        );
        assert!(terminal, "{label}");
        assert_eq!(request_count, 2, "{label}");
        assert_eq!(finished, Some(ActivityOutcome::Completed), "{label}");
        assert_eq!(replay.as_deref(), Some(native_output), "{label}");
        let profile =
            profile.expect("the successful native edit retains a typed availability state");
        let profile_result = profile.result.as_ref().unwrap();
        assert_eq!(
            profile_result["content"][0]["text"], native_output,
            "{label}"
        );
        let evidence = FilePublicationEvidence::from_snapshot(
            profile_result["publicationEvidence"].as_str().unwrap(),
        )
        .expect("bounded unavailable evidence");
        assert_eq!(evidence.path(), "src/file.txt", "{label}");
        assert_eq!(
            evidence.state(),
            &FilePublicationEvidenceState::Unavailable {
                reason: expected_reason.unwrap(),
            },
            "{label}"
        );
        assert!(!profile.plain_text.contains(before), "{label}");
        assert!(!profile.plain_text.contains(after), "{label}");
    }

    for (label, arguments, admission_mode) in [
        (
            "redacted admitted arguments",
            arguments.clone(),
            EditAdmissionMode::RedactArguments,
        ),
        (
            "admitted path mismatch",
            arguments.replace("src/file.txt", "other.txt"),
            EditAdmissionMode::Preserve,
        ),
    ] {
        let result =
            ToolExecutionResult::new(ToolExecutionOutcome::Completed, native_output, false)
                .with_retained_output(complete_profile.clone(), false);
        let (profile, finished, replay, request_count, terminal) = run_edit(
            arguments,
            result,
            Some(4096),
            admission_mode,
            20 * 1024 * 1024,
        );
        assert!(terminal, "{label}");
        assert_eq!(request_count, 2, "{label}");
        assert_eq!(finished, Some(ActivityOutcome::Completed), "{label}");
        assert_eq!(replay.as_deref(), Some(native_output), "{label}");
        let profile = profile.expect("completed native edit output");
        let profile_result = profile.result.as_ref().unwrap();
        assert_eq!(
            profile_result["content"][0]["text"], native_output,
            "{label}"
        );
        assert!(
            !profile_result
                .as_object()
                .unwrap()
                .contains_key("publicationEvidence"),
            "{label}"
        );
        assert!(
            !profile_result
                .as_object()
                .unwrap()
                .contains_key("retainedOutput"),
            "{label}"
        );
        assert!(!profile.plain_text.contains(before), "{label}");
        assert!(!profile.plain_text.contains(after), "{label}");
        assert!(
            !profile.plain_text.contains(complete_profile.as_str()),
            "{label}"
        );
    }

    let mismatched_receipt = r#"{"path":"other.txt","status":"ok","replacements":1}"#;
    let result =
        ToolExecutionResult::new(ToolExecutionOutcome::Completed, mismatched_receipt, false)
            .with_retained_output(complete_profile.clone(), false);
    let (profile, finished, replay, request_count, terminal) = run_edit(
        arguments.clone(),
        result,
        Some(4096),
        EditAdmissionMode::Preserve,
        20 * 1024 * 1024,
    );
    assert!(terminal, "receipt path mismatch");
    assert_eq!(request_count, 2, "receipt path mismatch");
    assert_eq!(
        finished,
        Some(ActivityOutcome::Completed),
        "receipt path mismatch"
    );
    assert_eq!(replay.as_deref(), Some(mismatched_receipt));
    let profile = profile.expect("completed native edit output");
    let profile_result = profile.result.as_ref().unwrap();
    assert_eq!(profile_result["content"][0]["text"], mismatched_receipt);
    assert!(
        !profile_result
            .as_object()
            .unwrap()
            .contains_key("publicationEvidence")
    );
    assert!(
        !profile_result
            .as_object()
            .unwrap()
            .contains_key("retainedOutput")
    );
    assert!(!profile.plain_text.contains(before));
    assert!(!profile.plain_text.contains(after));

    let truncated_receipt =
        ToolExecutionResult::new(ToolExecutionOutcome::Completed, native_output, true)
            .with_retained_output(complete_profile.clone(), false);
    let (profile, finished, replay, request_count, terminal) = run_edit(
        arguments.clone(),
        truncated_receipt,
        Some(4096),
        EditAdmissionMode::Preserve,
        20 * 1024 * 1024,
    );
    assert!(terminal, "truncated receipt");
    assert_eq!(request_count, 2, "truncated receipt");
    assert_eq!(
        finished,
        Some(ActivityOutcome::Completed),
        "truncated receipt"
    );
    let profile = profile.expect("completed native edit output");
    let profile_result = profile.result.as_ref().unwrap();
    assert_eq!(profile_result["truncated"], true);
    assert!(
        !profile_result
            .as_object()
            .unwrap()
            .contains_key("publicationEvidence")
    );
    assert_eq!(
        replay.as_deref(),
        profile_result["content"][0]["text"].as_str()
    );
    assert!(!profile.plain_text.contains(before));
    assert!(!profile.plain_text.contains(after));

    let failed = ToolExecutionResult::new(ToolExecutionOutcome::Interrupted, native_output, false);
    let (profile, finished, replay, _, terminal) = run_edit(
        arguments.clone(),
        failed,
        Some(4096),
        EditAdmissionMode::Preserve,
        20 * 1024 * 1024,
    );
    assert!(terminal);
    assert_eq!(finished, Some(ActivityOutcome::Interrupted));
    assert_eq!(replay.as_deref(), Some(native_output));
    let interrupted_result = profile.unwrap().result.unwrap();
    assert!(
        !interrupted_result
            .as_object()
            .unwrap()
            .contains_key("publicationEvidence")
    );
}

// 최종 ToolOutput이 넘치면 원본 편집 인자를 생략해도 path와 typed unavailable 상태를 보존한다.
#[test]
fn edit_publication_snapshot_overflow_keeps_completed_receipt_and_unavailable_path() {
    use serde_json::json;
    use yo_core::{
        ActivityOutcome, FilePublicationEvidence, FilePublicationEvidenceState,
        FilePublicationEvidenceUnavailableReason, ToolOutput,
    };

    let native_output = r#"{"path":"large.txt","status":"ok","replacements":1}"#;
    let before = "b".repeat(800_000);
    let after = "a".repeat(200_000);
    let captured_bytes = before.len() + after.len();
    assert_eq!(captured_bytes, 1_000_000);
    let retained = FilePublicationEvidence::complete("large.txt", before, after)
        .unwrap()
        .to_snapshot()
        .unwrap();
    assert!(retained.len() <= FilePublicationEvidence::MAX_SNAPSHOT_BYTES);
    let arguments = json!({
        "path": "large.txt",
        "edits": [{"oldText": "x".repeat(16_000_000), "newText": "y"}]
    })
    .to_string();
    assert!(arguments.len() < 16 * 1024 * 1024);
    assert!(arguments.len() + captured_bytes > ToolOutput::MAX_SNAPSHOT_BYTES);
    let (profile, finished, replay, request_count, terminal) = run_large_publication_edit(
        arguments,
        ToolExecutionResult::new(ToolExecutionOutcome::Completed, native_output, false)
            .with_retained_output(retained, false),
    );
    assert!(terminal);
    assert_eq!(request_count, 2);
    assert_eq!(finished, Some(ActivityOutcome::Completed));
    assert_eq!(replay.as_deref(), Some(native_output));
    let profile = profile.expect("snapshot-capacity profile");
    assert_eq!(
        profile.arguments.as_ref().unwrap(),
        &json!({"path":"large.txt"})
    );
    assert!(!profile.plain_text.contains("xxxxx"));
    let result = profile.result.as_ref().unwrap();
    assert_eq!(result["content"][0]["text"], native_output);
    let evidence =
        FilePublicationEvidence::from_snapshot(result["publicationEvidence"].as_str().unwrap())
            .unwrap();
    assert_eq!(evidence.path(), "large.txt");
    assert_eq!(
        evidence.state(),
        &FilePublicationEvidenceState::Unavailable {
            reason: FilePublicationEvidenceUnavailableReason::SnapshotCapacity,
        }
    );
    let serialized = profile.to_snapshot().unwrap();
    assert!(serialized.len() <= ToolOutput::MAX_SNAPSHOT_BYTES);
    assert!(!serialized.contains("xxxxxxxxxx"));
}

fn run_large_publication_edit(
    arguments: String,
    result: ToolExecutionResult,
) -> (
    Option<yo_core::ToolOutput>,
    Option<yo_core::ActivityOutcome>,
    Option<String>,
    usize,
    bool,
) {
    use serde_json::json;
    use yo_core::{
        ActivityUpdate, BackendPoll, FilePublicationEvidence, TOOL_SCHEMA_DIALECT,
        ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolRegistry, ToolSemanticAdmission,
    };

    struct Host(ToolExecutionResult);
    impl ToolExecutionHost for Host {
        fn identity(&self) -> &str {
            "native-edit-host-v1"
        }
        fn is_available(&self, _: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            request: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            assert_eq!(request.call.call_id(), "edit-call");
            assert_eq!(
                request.maximum_retained_output_bytes,
                Some(FilePublicationEvidence::MAX_SNAPSHOT_BYTES)
            );
            Ok(Box::new(MockExecution {
                result: Some(self.0.clone()),
            }))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }
    struct Admission;
    impl ToolSemanticAdmission for Admission {
        fn admit_arguments(
            &self,
            _: &ToolDefinition,
            arguments: &str,
        ) -> Result<String, yo_core::ToolSemanticAdmissionError> {
            Ok(arguments.to_owned())
        }
        fn admit_output(
            &self,
            _: &ToolDefinition,
            output: &str,
        ) -> Result<String, yo_core::ToolSemanticAdmissionError> {
            Ok(output.to_owned())
        }
    }
    let registry = ToolRegistry::new([ToolDefinition::new(
        ToolId::new("edit-file").unwrap(),
        "edit_file",
        "edits exact text in one workspace file",
        TOOL_SCHEMA_DIALECT,
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": {"type": "string"},
                            "newText": {"type": "string"}
                        },
                        "required": ["oldText", "newText"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        }),
        ToolEffect::WorkspaceWrite,
        ToolApprovalRequirement::Automatic,
    )
    .unwrap()
    .with_argument_byte_limit(18 * 1024 * 1024)
    .unwrap()])
    .unwrap()
    .freeze();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let rounds = vec![
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r1".to_owned(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index: 0,
                item_id: "edit-item".to_owned(),
                call_id: "edit-call".to_owned(),
                name: "edit_file".to_owned(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index: 0,
                item_id: "edit-item".to_owned(),
                call_id: "edit-call".to_owned(),
                name: "edit_file".to_owned(),
                arguments,
            },
            completed("r1"),
        ],
        vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "r2".to_owned(),
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
        registry,
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(Admission)),
            Box::new(Host(result)),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig {
            maximum_tool_output_bytes: 4096,
            maximum_tool_argument_bytes: 16 * 1024 * 1024,
            maximum_retained_tool_output_bytes: Some(FilePublicationEvidence::MAX_SNAPSHOT_BYTES),
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
            input: UserInput::from("inspect edit"),
        })
        .unwrap();
    let mut observed_activity = None;
    let mut profile = None;
    let mut finished = None;
    let mut terminal = false;
    for _ in 0..200 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) => {
                if let Some(output) = yo_core::ToolOutput::from_snapshot(&text)
                    .filter(|output| output.tool == "edit_file" && output.result.is_some())
                {
                    observed_activity = Some(activity);
                    profile = Some(output);
                }
            },
            BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome })
                if observed_activity == Some(activity) =>
            {
                finished = Some(outcome);
            },
            BackendPoll::Event(BackendEvent::TurnFinished { .. })
            | BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                terminal = true;
                break;
            },
            _ => {},
        }
    }
    let requests = requests.lock().unwrap();
    let replay_output = requests.get(1).and_then(|request| {
        request.input().iter().find_map(|item| match item {
            ModelConnectorInputItem::FunctionCallOutput { call_id, output }
                if call_id == "edit-call" =>
            {
                Some(output.clone())
            },
            _ => None,
        })
    });
    (profile, finished, replay_output, requests.len(), terminal)
}
