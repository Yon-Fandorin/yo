use super::*;
use crate::VersionedProfileId;

// Durable pressure JSON의 생산자와 소비자가 하나의 닫힌 typed 문법을 공유하고
// unknown field나 잘못된 policy 경계를 조용히 표시하지 않음을 검증합니다.
#[test]
fn context_pressure_snapshot_round_trips_only_the_closed_shape() {
    let observation =
        ContextPressureObservation::new(86, 100, 85, 90, ContextPressureDecision::Admit).unwrap();
    let snapshot = observation.to_snapshot_json();

    assert_eq!(
        ContextPressureObservation::from_snapshot_json(&snapshot),
        Some(observation)
    );
    assert!(
        ContextPressureObservation::from_snapshot_json(
            &snapshot.replace("\"input_tokens\":86", "\"extra\":0,\"input_tokens\":86")
        )
        .is_none()
    );
    assert!(
        ContextPressureObservation::new(86, 100, 90, 90, ContextPressureDecision::Admit).is_err()
    );
}

// 이미지가 0개라도 advisory policy와 reserve 0을 v2에 보존하고 legacy scalar를 섞지 않는다.
#[test]
fn image_pressure_preserves_advisory_metadata_even_without_images() {
    let accounting = ContextAccounting::new(
        ContextAccountingQuality::AdvisoryEstimate,
        VersionedProfileId::new(KIMI_CODE_IMAGE_ACCOUNTING_PROFILE).unwrap(),
        86,
        0,
    )
    .unwrap();
    let pressure = ContextPressureObservation::new(86, 100, 85, 90, ContextPressureDecision::Admit)
        .unwrap()
        .with_accounting(accounting)
        .unwrap();
    let wire = pressure.to_snapshot_json();
    assert_eq!(
        wire,
        r#"{"schema":"yo.context-pressure/v2alpha1","accounting":{"quality":"advisory_estimate","policy":"kimi-code-image-advisory/v1","input_estimate":86,"reserve_tokens":0},"input_token_limit":100,"warning_percent":85,"trigger_percent":90,"decision":"admit"}"#
    );
    assert_eq!(
        ContextPressureObservation::from_snapshot_json(&wire),
        Some(pressure)
    );
    for malformed in [
        wire.replace("\"accounting\":", "\"input_tokens\":86,\"accounting\":"),
        wire.replace("advisory_estimate", "exact"),
        wire.replace("\"reserve_tokens\":0", "\"reserve_tokens\":null"),
        wire.replace(
            "\"quality\":",
            "\"quality\":\"advisory_estimate\",\"quality\":",
        ),
    ] {
        assert!(ContextPressureObservation::from_snapshot_json(&malformed).is_none());
    }
    let qwen_wire = wire.replace(
        KIMI_CODE_IMAGE_ACCOUNTING_PROFILE,
        QWENCLOUD_GENERAL_IMAGE_ACCOUNTING_PROFILE,
    );
    let qwen = ContextPressureObservation::from_snapshot_json(&qwen_wire).unwrap();
    assert_eq!(qwen.to_snapshot_json(), qwen_wire);
    assert_eq!(qwen.accounting().unwrap().reserve_tokens(), 0);
    assert!(
        ContextPressureObservation::from_snapshot_json(
            &qwen_wire.replace("advisory_estimate", "verified_upper_bound")
        )
        .is_none()
    );
}

// 완전한 요청 planning 합의 overflow·미승인 policy·호환되지 않는 quality는 거절한다.
#[test]
fn image_accounting_rejects_unknown_policy_quality_and_overflow() {
    let policy = || VersionedProfileId::new(KIMI_CODE_IMAGE_ACCOUNTING_PROFILE).unwrap();
    assert!(
        ContextAccounting::new(
            ContextAccountingQuality::AdvisoryEstimate,
            policy(),
            u64::MAX,
            1024
        )
        .is_err()
    );
    assert!(ContextAccounting::new(ContextAccountingQuality::Exact, policy(), 1, 0).is_err());
    assert!(
        ContextAccounting::new(ContextAccountingQuality::VerifiedUpperBound, policy(), 1, 0)
            .is_err()
    );
    assert!(
        ContextAccounting::new(
            ContextAccountingQuality::AdvisoryEstimate,
            VersionedProfileId::new("unknown/v1").unwrap(),
            1,
            0
        )
        .is_err()
    );
    assert!(
        ContextAccounting::new(
            ContextAccountingQuality::AdvisoryEstimate,
            policy(),
            u64::MAX,
            0
        )
        .is_ok()
    );
}
