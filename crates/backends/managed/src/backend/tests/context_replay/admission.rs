use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, BackendCommandEvidence, BackendEvent, BackendFailureKind,
    ToolApprovalRequirement, TurnOutcome, TurnRef, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, drain_until_turn,
        event_rounds, mock_tokenization_payload, registry, turn,
    },
    fixtures::{RecordingTokenCounter, SequenceTokenCounter, pressure_at_hard_limit_config},
};
use crate::backend::{NativeModelBackend, NativeModelBackendServices};

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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(100)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        pressure_at_hard_limit_config(),
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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(RecordingTokenCounter {
                input_tokens: 95,
                payloads: Arc::clone(&payloads),
            }),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        pressure_at_hard_limit_config(),
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
        mock_tokenization_payload(&requests[0], "qwen3.8max")["max_output_tokens"],
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
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new(
                [95, 100, 99],
                Arc::clone(&payloads),
            )),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        pressure_at_hard_limit_config(),
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
        mock_tokenization_payload(&requests[0], "qwen3.8max")["max_output_tokens"],
        1
    );
}
