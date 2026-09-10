use std::sync::{Arc, Mutex};

use yo_core::{ToolApprovalRequirement, ToolOutput, admit_standard_complete_binding};

use super::{
    super::{
        NativeModelBackendConfig, NativeModelBackendServices,
        tests::support::{
            ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, context_profile,
            event_rounds, registry,
        },
    },
    NativeModelBackend, TOOL_TRUNCATION_MARKER, bounded_output,
};

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

    for invalid in [
        NativeModelBackendConfig {
            maximum_tool_output_bytes: TOOL_TRUNCATION_MARKER.len() - 1,
            ..NativeModelBackendConfig::default()
        },
        NativeModelBackendConfig {
            maximum_retained_tool_output_bytes: Some(0),
            ..NativeModelBackendConfig::default()
        },
        NativeModelBackendConfig {
            maximum_retained_tool_output_bytes: Some(ToolOutput::MAX_SNAPSHOT_BYTES + 1),
            ..NativeModelBackendConfig::default()
        },
    ] {
        assert!(
            NativeModelBackend::with_connector(
                Box::new(MockConnector {
                    rounds: event_rounds(Vec::new()),
                    requests: Arc::new(Mutex::new(Vec::new())),
                }),
                binding(),
                registry(ToolApprovalRequirement::Automatic),
                NativeModelBackendServices::new(
                    Box::new(admit_standard_complete_binding),
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
}
