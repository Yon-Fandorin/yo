use std::env;

use serde_json::json;

use super::{alpha2, alpha3, alpha4};
use crate::validation_summary::{
    current_reusable_context, current_toolchain_hash, model::ReuseContext,
    reuse::verify_current_reuse_context,
};

// commit fast path는 현재 platform/toolchain까지 다시 맞춘 context-bound summary만
// 사용할 수 있고 frozen v1alpha2 evidence는 정상 gate 증거여도 hook 대체는 못 합니다.
#[test]
fn commit_fast_path_requires_current_context_bound_evidence() {
    let candidate = "a".repeat(40);
    assert!(current_reusable_context(&alpha3("hk", &candidate)).unwrap());
    assert!(current_reusable_context(&alpha4("hk", &candidate)).unwrap());
    assert!(!current_reusable_context(&alpha2("hk", &candidate)).unwrap());

    let mut stale: serde_json::Value = serde_json::from_slice(&alpha3("hk", &candidate)).unwrap();
    stale["reuse_context"]["platform_os"] = json!("changed-os");
    assert!(!current_reusable_context(&serde_json::to_vec(&stale).unwrap()).unwrap());
}

// gate의 재사용 시점에는 현재 platform과 toolchain을 다시 관찰하므로 이전 실행의
// context가 달라지면 ancestor와 argv가 같아도 재사용할 수 없다.
#[test]
fn alpha3_reuse_context_invalidates_platform_or_toolchain_changes() {
    let current = ReuseContext {
        schema: "yo.validation-reuse-context/v1alpha1".to_owned(),
        platform_os: env::consts::OS.to_owned(),
        platform_arch: env::consts::ARCH.to_owned(),
        toolchain_hash: current_toolchain_hash().unwrap(),
        external_state: "none-declared".to_owned(),
    };
    verify_current_reuse_context(&current).unwrap();

    let changed_platform = ReuseContext {
        platform_os: "changed-os".to_owned(),
        ..current
    };
    assert!(
        verify_current_reuse_context(&changed_platform)
            .unwrap_err()
            .contains("platform changed")
    );
}
