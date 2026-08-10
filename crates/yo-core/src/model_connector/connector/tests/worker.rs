use std::time::{Duration, Instant};

use tokio::sync::mpsc as async_mpsc;

use super::{super::worker::send_event, support::dummy_event};
use crate::model_connector::{
    ConnectorFailureKind, ResponsesCancellation, ResponsesConnectorLimits,
};

// bounded event queue가 가득 찬 동안에도 전달 대기는 total request deadline을
// 벗어나지 않고 typed Timeout으로 끝나는지 검증합니다.
#[test]
fn event_backpressure_obeys_the_total_request_deadline() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sender, _receiver) = async_mpsc::channel(1);
    sender.try_send(dummy_event("first")).unwrap();
    let limits = ResponsesConnectorLimits {
        total_request_timeout: Duration::from_millis(25),
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
    assert!(started.elapsed() >= Duration::from_millis(10));
}
