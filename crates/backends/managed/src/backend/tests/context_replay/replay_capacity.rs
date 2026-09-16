use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    ActivityKind, ActivityRequestRef, ActivityResponse, AgentCommand, ApprovalDecision,
    BackendCommandEvidence, BackendEvent, BackendFailureKind, BackendPoll, ModelConnectorEvent,
    ToolApprovalRequirement, TurnOutcome, TurnRef, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, binding, completed,
        drain_until_turn, event_rounds, mock_tokenization_payload, registry, turn,
    },
    fixtures::{
        FailingStartHost, SequenceTokenCounter, fill_current_delta_to_item_limit,
        pressure_at_hard_limit_config, retain_empty_assistant_items, tool_call_round,
    },
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new([1, 100], Arc::clone(&payloads))),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
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
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
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
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(FailingStartHost),
            Box::new(FixedTokenCounter(1)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
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
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
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
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
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
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(input_tokens)),
            ),
            yo_core::ModelContextProfile::with_optional_output_limit(
                100,
                None,
                "test-tokenizer/v1",
            )
            .unwrap(),
            pressure_at_hard_limit_config(),
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
                mock_tokenization_payload(request, "qwen3.8max")
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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
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
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    let error = backend
        .execute_command(AgentCommand::StartTurn {
            turn: next_turn,
            input: UserInput::from("retry"),
        })
        .unwrap_err();
    assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
}
