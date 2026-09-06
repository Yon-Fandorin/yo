use std::sync::{Arc, Mutex};

use yo_backend::BackendAdapter;
use yo_core::{
    AgentCommand, BackendFailureKind, ModelConnectorEvent, ModelContextProfile, ModelReplayItem,
    ToolApprovalRequirement, UserInput,
};

use super::super::{
    NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
    tests::support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, event_rounds,
        kimi::{kimi_admission, kimi_binding, kimi_profile, private_envelope, visible_message},
        registry, turn,
    },
};

// backend는 Kimi private object 단독 크기가 아니라 같은 turn의 visible semantic item과
// contract를 합친 canonical replay delta를 저장 전에 재며 exact 16 MiB는 받고 +1은 버립니다.
#[test]
fn native_backend_bounds_complete_semantic_and_private_replay_before_retention() {
    let mut backend = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        kimi_binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(kimi_admission),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        ModelContextProfile::new(1_048_576, 131_072, "utf8-bytes/v1").unwrap(),
        Some(kimi_profile()),
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
            input: UserInput::from("size the complete replay delta"),
        })
        .unwrap();

    let visible = "v".repeat(4 * 1024 * 1024);
    let mut state = backend.turn.take().unwrap();
    state.round_message_items.insert(0);
    state.round_messages.insert((0, 0), visible.clone());
    let item = |reasoning_bytes| ModelReplayItem::ProviderPrivateAssistant {
        envelope: private_envelope(&"r".repeat(reasoning_bytes), Some(&visible)),
    };

    let empty_private = item(0);
    let fixed_bytes = backend
        .prospective_replay_delta_encoded_len(&state, Some((1, &empty_private)))
        .unwrap();
    let accepted = super::ModelReplayDelta::MAX_ENCODED_BYTES - fixed_bytes;
    backend
        .ensure_replay_capacity_with_round_item(&state, Some((1, &item(accepted))))
        .expect("the exact complete replay boundary is admitted");

    let overflow = backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::ProviderPrivateAssistant {
                output_index: 1,
                envelope: private_envelope(&"r".repeat(accepted + 1), Some(&visible)),
                visible_projection: vec![visible_message(visible.clone())],
            },
        )
        .unwrap_err();
    assert_eq!(overflow.kind(), BackendFailureKind::ContextExhausted);
    assert!(state.round_replay.is_empty());

    backend
        .apply_response_event(
            &mut state,
            ModelConnectorEvent::ProviderPrivateAssistant {
                output_index: 1,
                envelope: private_envelope(&"r".repeat(accepted), Some(&visible)),
                visible_projection: vec![visible_message(visible)],
            },
        )
        .unwrap();
    assert!(state.round_replay.contains_key(&1));
}
