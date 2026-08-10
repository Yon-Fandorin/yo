use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendCommandEvidence, BackendEvent, BackendFailureKind,
        ModelConnectorEvent, ModelReplayContract, ModelReplayDelta, ModelReplayItem,
        ModelReplayRole, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
        ToolApprovalRequirement, TurnOutcome, TurnRef,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, binding, completed,
        drain_until_turn, event_rounds, registry, turn,
    },
};
use crate::UserInput;

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

// input count가 input limit에서 wire output cap을 뺀 값과 정확히 같으면 요청을 허용하고,
// 같은 cap이 tokenization payload와 실제 connector request에 포함되는지 검증합니다.
#[test]
fn admits_the_exact_input_boundary_with_the_configured_output_cap() {
    let requests = Arc::new(Mutex::new(Vec::new()));
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
            Box::new(FixedTokenCounter(90)),
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
        10
    );
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
    backend
        .replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            (0..4_096)
                .map(|_| ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: String::new(),
                    refusal: None,
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
