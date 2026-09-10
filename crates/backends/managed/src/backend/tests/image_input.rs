use std::sync::{Arc, Mutex};

use yo_backend::BackendAdapter;
use yo_core::{
    ActivityUpdate, AgentCommand, BackendCommandEvidence, BackendEvent, CompleteModelBinding,
    ContextPolicyChanged, ContextPressureObservation, ContextStrategy, InputImage,
    InputImageSnapshot, ModelConnectorInputItem, ToolRegistry, UserInput,
};

use super::support::{
    FixedTokenCounter, MockConnector, MockHost, event_rounds, kimi::kimi_admission, turn,
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

fn image_input(count: usize) -> UserInput {
    let snapshot: InputImageSnapshot = serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap();
    if count == 0 {
        return UserInput::new("text only");
    }
    UserInput::new("[image]".repeat(count))
        .with_images(
            (0..count)
                .map(|index| {
                    InputImage::new(index * 7..index * 7 + 7, 70, snapshot.clone()).unwrap()
                })
                .collect(),
        )
        .unwrap()
}

// 실제 모델 루프의 최종 요청 cap과 pressure는 같은 전체 추정치를 사용하고 N=0도 v2 정책을 유지한다.
#[test]
fn final_request_cap_and_pressure_use_one_complete_image_estimate() {
    let complete = CompleteModelBinding::from_durable_json(r#"{"provider":"kimi","account":"default","model":"k3-256k","connector":"kimi-chat-completions","base_url":"https://api.kimi.com/coding/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":262144,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{"thinking":{"type":"enabled","keep":"all"}},"tool_capability_policy":"no-tools/v1","replay_profile":"kimi-private-local-plaintext/v1","image_input_profile":"kimi-code-png-advisory/v1"}"#).unwrap();
    for image_count in [0, 1, 2] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let config = NativeModelBackendConfig {
            context_policy: ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                100,
                Some(10),
                Some(65536),
            )
            .unwrap(),
            ..NativeModelBackendConfig::default()
        };
        let mut backend = NativeModelBackend::with_connector_and_profile(
            Box::new(MockConnector {
                rounds: event_rounds(vec![vec![]]),
                requests: Arc::clone(&requests),
            }),
            complete.binding().clone(),
            ToolRegistry::default().freeze(),
            NativeModelBackendServices::new(
                Box::new(kimi_admission),
                None,
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(235000)),
            ),
            complete.profile().context().clone(),
            Some(complete.profile().clone()),
            config,
        )
        .unwrap();
        let turn = turn();
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn.session_id(),
            })
            .unwrap();
        let input = image_input(image_count);
        let expected_parts = input.model_parts();
        assert!(matches!(
            backend
                .execute_command(AgentCommand::StartTurn { turn, input })
                .unwrap(),
            BackendCommandEvidence::RequestAccepted(_)
        ));
        let estimate = 235000 + 2000 * image_count as u64;
        let reserve = if image_count == 0 { 0 } else { 1024 };
        {
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].max_output_tokens(),
                Some(262144 - estimate - reserve)
            );
            assert_eq!(requests[0].image_count(), image_count);
            if image_count > 0 {
                assert!(
                    matches!(&requests[0].input()[1], ModelConnectorInputItem::MultimodalUser { parts } if parts == &expected_parts)
                );
            }
        }
        let pressure = backend
            .events
            .iter()
            .find_map(|event| match event {
                BackendEvent::ActivityUpdated {
                    update: ActivityUpdate::TextSnapshot(text),
                    ..
                } => ContextPressureObservation::from_snapshot_json(text),
                _ => None,
            })
            .expect("warning emits typed context pressure");
        let accounting = pressure
            .accounting()
            .expect("selected image binding keeps v2 at N=0");
        assert_eq!(accounting.input_estimate(), estimate);
        assert_eq!(accounting.reserve_tokens(), reserve);
        assert_eq!(pressure.input_tokens(), estimate + reserve);
        backend.shutdown().unwrap();
    }
}
