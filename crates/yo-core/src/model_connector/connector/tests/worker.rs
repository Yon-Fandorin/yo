use std::time::{Duration, Instant};

use tokio::sync::mpsc as async_mpsc;

use super::{
    super::{failure::record_body_progress, worker::send_event},
    support::dummy_event,
};
use crate::model_connector::{
    ConnectorFailureKind, ResponsesCancellation, ResponsesConnectorLimits,
};

// bounded event queue가 가득 찬 동안 각 observation의 독립적인 delivery wait가
// agent absolute deadline 없이도 자체 timeout으로 끝나는지 검증합니다.
#[test]
fn event_backpressure_obeys_its_own_delivery_deadline() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sender, _receiver) = async_mpsc::channel(1);
    sender.try_send(dummy_event("first")).unwrap();
    let limits = ResponsesConnectorLimits {
        event_delivery_timeout: Duration::from_millis(25),
        ..ResponsesConnectorLimits::default()
    };
    let cancellation = ResponsesCancellation::new();
    let started = Instant::now();

    let error = runtime
        .block_on(send_event(
            &sender,
            &cancellation,
            started,
            &limits,
            dummy_event("second"),
        ))
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Timeout);
    assert!(error.message().contains("event delivery"));
    assert!(started.elapsed() >= Duration::from_millis(10));
}

// raw body progress 기록은 비어 있지 않은 chunk만 inactivity 시작점을 옮겨서,
// empty chunk가 무한히 들어와도 successful/error-body idle clock을 연장하지 못하게 합니다.
#[test]
fn only_non_empty_raw_body_chunks_reset_inactivity() {
    let original = Instant::now();
    let later = original + Duration::from_secs(1);
    let mut progress = original;

    record_body_progress(&mut progress, &[], later);
    assert_eq!(progress, original);

    record_body_progress(&mut progress, b": heartbeat\n\n", later);
    assert_eq!(progress, later);
}

// 기본 runtime policy는 transport와 handoff phase를 유한하게 두되 agent-owned absolute
// request deadline은 설정하지 않아 건강한 장기 stream을 wall clock으로 자르지 않습니다.
#[test]
fn default_policy_has_finite_phase_deadlines_without_an_absolute_deadline() {
    let limits = ResponsesConnectorLimits::default();

    assert_eq!(limits.connect_timeout, Duration::from_secs(30));
    assert_eq!(limits.response_header_timeout, Duration::from_secs(5 * 60));
    assert_eq!(limits.stream_idle_timeout, Duration::from_secs(5 * 60));
    assert_eq!(limits.error_body_idle_timeout, Duration::from_secs(30));
    assert_eq!(limits.event_delivery_timeout, Duration::from_secs(5 * 60));
    assert_eq!(limits.absolute_request_timeout, None);
}

// optional absolute deadline을 실제로 설정한 경우 0은 phase deadline과 같은 configuration
// failure로 거절하여 `Some(0)`이 즉시 만료나 무제한이라는 두 의미로 갈리지 않게 합니다.
#[test]
fn rejects_a_zero_optional_absolute_request_deadline() {
    let limits = ResponsesConnectorLimits {
        absolute_request_timeout: Some(Duration::ZERO),
        ..ResponsesConnectorLimits::default()
    };

    let error = limits.validate().unwrap_err();
    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
    assert!(error.message().contains("non-zero"));
}
