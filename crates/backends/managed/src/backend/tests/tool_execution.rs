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
            if std::mem::replace(&mut self.sent, true) {
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
