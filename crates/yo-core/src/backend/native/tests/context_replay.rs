use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use super::{
    super::{
        ActivityKind, ActivityRequestRef, ActivityResponse, AgentBackend, AgentCommand,
        ApprovalDecision, BackendCommandEvidence, BackendEvent, BackendFailureKind, BackendPoll,
        ModelConnectorEvent, ModelReplayContract, ModelReplayDelta, ModelReplayItem,
        ModelReplayRole, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
        ToolApprovalRequirement, ToolExecution, ToolExecutionHost, ToolExecutionRequest,
        TurnOutcome, TurnRef,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, binding, completed,
        drain_until_turn, event_rounds, registry, turn,
    },
};
use crate::{ToolExecutionError, ToolId, UserInput};

struct RecordingTokenCounter {
    input_tokens: u64,
    payloads: Arc<Mutex<Vec<serde_json::Value>>>,
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
        Err(ToolExecutionError::new("injected start failure"))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

struct SequenceTokenCounter {
    input_tokens: Mutex<VecDeque<u64>>,
    payloads: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl SequenceTokenCounter {
    fn new(
        input_tokens: impl IntoIterator<Item = u64>,
        payloads: Arc<Mutex<Vec<serde_json::Value>>>,
    ) -> Self {
        Self {
            input_tokens: Mutex::new(input_tokens.into_iter().collect()),
            payloads,
        }
    }
}

impl crate::ModelTokenCounter for SequenceTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, crate::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        Ok(self
            .input_tokens
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test declared every exact payload count"))
    }
}

fn tool_call_round() -> Vec<ModelConnectorEvent> {
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
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        completed("tool"),
    ]
}

fn retain_empty_assistant_items(backend: &mut NativeModelBackend, count: usize) {
    backend
        .replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            (0..count)
                .map(|_| ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: String::new(),
                    refusal: None,
                })
                .collect(),
        ))
        .unwrap();
}

fn fill_current_delta_to_item_limit(backend: &mut NativeModelBackend) {
    let delta = &mut backend
        .turn
        .as_mut()
        .expect("the first model request opened a Turn")
        .delta;
    assert_eq!(delta.len(), 1);
    delta.extend((1..4_095).map(|_| ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: String::new(),
        refusal: None,
    }));
    assert_eq!(delta.len(), 4_095);
}

impl crate::ModelTokenCounter for RecordingTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, crate::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        Ok(self.input_tokens)
    }
}

// 100-token 입력은 100-token limit 안에 양수 output cap을 하나도 남기지 않으므로 remote
// call 없이 code=context_exhausted인 Failed Turn을 남기고 다음 Turn도 거절합니다.
#[test]
fn context_exhaustion_finishes_non_resumably_and_latches_the_binding() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(100)),
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
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("context exhaustion must finish as a failed non-resumable Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert!(requests.lock().unwrap().is_empty());

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

// 95-token 입력에서 hard max 10은 넘치지만 계산된 cap 5는 정확히 100에 맞으므로, cap 10과
// cap 5 payload를 각각 count한 뒤 최종 cap 5 요청만 connector에 전달합니다.
#[test]
fn recounts_and_dispatches_the_exact_smaller_output_cap() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(RecordingTokenCounter {
                input_tokens: 95,
                payloads: Arc::clone(&payloads),
            }),
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
                input: UserInput::from("exact boundary"),
            })
            .unwrap(),
        BackendCommandEvidence::RequestAccepted(_)
    ));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].tokenization_payload("qwen3.8max")["max_output_tokens"],
        5
    );
    let payloads = payloads.lock().unwrap();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0]["max_output_tokens"], 10);
    assert_eq!(payloads[1]["max_output_tokens"], 5);
}

// count 결과가 차례로 95, 100, 99이면 100-token limit과 hard max 10에서 payload cap은
// 10, 5, 1로 엄격히 감소하고, 세 번째 cap 1 payload만 connector에 한 번 전달됩니다.
#[test]
fn bounded_selector_uses_at_most_three_strictly_decreasing_exact_counts() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new(
                [95, 100, 99],
                Arc::clone(&payloads),
            )),
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
                input: UserInput::from("three exact counts"),
            })
            .unwrap(),
        BackendCommandEvidence::RequestAccepted(_)
    ));
    let payloads = payloads.lock().unwrap();
    assert_eq!(payloads.len(), 3);
    assert_eq!(payloads[0]["max_output_tokens"], 10);
    assert_eq!(payloads[1]["max_output_tokens"], 5);
    assert_eq!(payloads[2]["max_output_tokens"], 1);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].tokenization_payload("qwen3.8max")["max_output_tokens"],
        1
    );
}

// 첫 요청은 input 1 + cap 10으로 성공하고 자동 도구 결과까지 누적되지만, 두 번째 요청의
// exact input 100은 양수 cap을 남기지 않습니다. connector request는 1건에서 멈추고 현재
// Turn은 code=context_exhausted로 실패하며 같은 binding의 다음 Turn도 거절됩니다.
#[test]
fn post_tool_round_exhaustion_stops_before_a_second_dispatch_and_latches() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new([1, 100], Arc::clone(&payloads))),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("run one tool"),
        })
        .unwrap();

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("the second-round context overflow must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(payloads.lock().unwrap().len(), 2);

    let next_turn = TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// 첫 요청 뒤 current delta를 user 포함 4095 items로 채운 seam에서 model function call은
// 4096에 정확히 맞습니다. 성공한 tool output은 4097번째 item이라 finish_tool이 직접
// ContextExhausted를 반환하고, poll_tool은 typed failure와 latch를 보존합니다.
#[test]
fn successful_tool_output_replay_overflow_uses_the_typed_failure_path() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("fill replay with tool output"),
        })
        .unwrap();
    fill_current_delta_to_item_limit(&mut backend);

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("tool-output replay exhaustion must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let next_turn = TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// 첫 요청 뒤 current delta를 user 포함 4095 items로 채우면 function call은 4096에 맞습니다.
// execution host의 동기 start 실패로 backend가 만드는 failed tool output은 4097번째 item이라
// dispatch의 start-tool 오류 경로도 code=context_exhausted인 Failed Turn과 latch를 보존합니다.
#[test]
fn synchronous_tool_start_failure_replay_overflow_is_typed_and_latched() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(FailingStartHost),
            Box::new(FixedTokenCounter(1)),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("start the failing tool"),
        })
        .unwrap();
    fill_current_delta_to_item_limit(&mut backend);

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("synchronous tool-start replay exhaustion must fail the Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let next_turn = TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// 첫 요청 뒤 current delta를 user 포함 4095 items로 채우면 approval 대상 function call은
// 4096에 맞습니다. decline tool output은 4097번째 item이라 approval response command가 Turn을
// 잃지 않고 code=context_exhausted와 latch를 기록하며 request는 첫 1건에서 멈춥니다.
#[test]
fn approval_decline_replay_overflow_is_typed_and_latched() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Required),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("request approval"),
        })
        .unwrap();
    fill_current_delta_to_item_limit(&mut backend);
    let request = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            }) => break ActivityRequestRef::new(activity, request_id),
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before approval request"),
        }
    };
    assert!(matches!(
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(ApprovalDecision::Declined),
            })
            .unwrap(),
        BackendCommandEvidence::None
    ));

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("approval-decline replay exhaustion must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let next_turn = TurnRef::new(
        turn().session_id(),
        crate::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// output hard max가 unknown인 profile은 99-token payload에서 제한 필드를 생략한 채 한 번
// count하고 dispatch하지만, 같은 payload가 100-token limit과 같으면 remote call 없이 실패합니다.
#[test]
fn unknown_output_cap_uses_strict_input_boundary_and_omits_the_field() {
    for (input_tokens, admitted) in [(99, true), (100, false)] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(if admitted {
                    vec![Vec::new()]
                } else {
                    Vec::new()
                }),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(input_tokens)),
            ),
            crate::ModelContextProfile::with_optional_output_limit(100, None, "test-tokenizer/v1")
                .unwrap(),
            NativeModelBackendConfig::default(),
        )
        .unwrap();
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        let evidence = backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("unknown cap"),
            })
            .unwrap();

        assert_eq!(
            matches!(evidence, BackendCommandEvidence::RequestAccepted(_)),
            admitted
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), usize::from(admitted));
        if let Some(request) = requests.first() {
            assert!(
                request
                    .tokenization_payload("qwen3.8max")
                    .get("max_output_tokens")
                    .is_none()
            );
        }
    }
}

// retained prefix가 이미 4096 items이면 새 user item을 더한 누적 replay가 dispatch 전에
// 거절되고 connector request는 0건이며 Turn에는 typed context exhaustion이 남습니다.
#[test]
fn cumulative_replay_capacity_is_checked_before_dispatch() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        crate::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    retain_empty_assistant_items(&mut backend, 4_096);
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("overflow before dispatch"),
        })
        .unwrap();

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("cumulative replay exhaustion must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert!(requests.lock().unwrap().is_empty());
}

// 완료 응답을 replay에 더하는 순간 누적 한도를 넘더라도 실패 기록이나 재개 Anchor를
// 만들지 않고 현재 Turn을 완결한 뒤 같은 binding의 추가 호출을 차단한다.
#[test]
fn replay_exhaustion_finishes_non_resumably_and_latches_the_binding() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "full".to_owned(),
            },
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "answer".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("full"),
        ]],
        ToolApprovalRequirement::Automatic,
        starts,
    );
    retain_empty_assistant_items(&mut backend, 4_095);
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
