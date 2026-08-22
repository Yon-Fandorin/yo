use super::*;

// Core를 감싸는 타입도 foundation envelope의 비공개 payload를 Debug 출력에 노출하지 않습니다.
#[test]
fn provider_private_debug_is_redacted_through_core_enclosing_types() {
    let private = "private-reasoning-sentinel";
    let envelope = ProviderPrivateReplayEnvelope::new(
        "provider.private/v1",
        br#"{"private":"private-reasoning-sentinel"}"#.to_vec(),
    )
    .unwrap();
    let event = crate::ResponsesEvent::ProviderPrivateAssistant {
        output_index: 1,
        envelope: envelope.clone(),
        visible_projection: Vec::new(),
    };
    let input = crate::ModelConnectorInputItem::ProviderPrivateAssistant {
        envelope: envelope.clone(),
    };

    for rendered in [format!("{event:?}"), format!("{input:?}")] {
        assert!(!rendered.contains(private), "{rendered}");
        assert!(rendered.contains("payload_bytes"));
    }
}
