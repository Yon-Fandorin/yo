use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

use tokio::sync::mpsc as async_mpsc;
use yo_core::{ModelConnectorCancellation, ModelConnectorLimits, ModelConnectorPoll};

use super::{
    super::{ConnectorStream, worker::send_event},
    support::dummy_event,
};

// worker outcome이 먼저 보이더라도 event sender가 닫힐 때까지 queue를 계속 drain하여
// outcome과 동시에 enqueue된 마지막 terminal event를 잃지 않는지 검증합니다.
#[test]
fn drains_every_event_before_observing_the_worker_outcome() {
    let (event_sender, event_receiver) = async_mpsc::channel(1);
    let (outcome_sender, outcome_receiver) = mpsc::sync_channel(1);
    outcome_sender.send(Ok(())).unwrap();
    let cancellation = ModelConnectorCancellation::new();
    let mut stream = ConnectorStream {
        receiver: event_receiver,
        outcome: outcome_receiver,
        cancellation,
        worker: None,
        closed: false,
    };

    assert_eq!(stream.poll().unwrap(), ModelConnectorPoll::Pending);
    event_sender.try_send(dummy_event("terminal")).unwrap();
    drop(event_sender);
    assert!(matches!(
        stream.poll().unwrap(),
        ModelConnectorPoll::Event(yo_core::ModelConnectorEvent::ResponseCreated { response_id })
            if response_id == "terminal"
    ));
    assert_eq!(stream.poll().unwrap(), ModelConnectorPoll::Closed);
}

// stream Drop은 가득 찬 event queue에서 대기 중인 worker를 취소한 뒤 join하여
// detached thread나 포화 queue shutdown deadlock을 남기지 않는지 검증합니다.
#[test]
fn drop_cancels_and_joins_a_worker_blocked_by_event_backpressure() {
    let (event_sender, event_receiver) = async_mpsc::channel(1);
    event_sender.try_send(dummy_event("first")).unwrap();
    let (outcome_sender, outcome_receiver) = mpsc::sync_channel(1);
    let cancellation = ModelConnectorCancellation::new();
    let worker_cancellation = cancellation.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = Arc::clone(&finished);
    let worker = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(send_event(
            &event_sender,
            &worker_cancellation,
            Instant::now(),
            &ModelConnectorLimits::default(),
            dummy_event("second"),
        ));
        let _ = outcome_sender.send(result);
        worker_finished.store(true, Ordering::Release);
    });
    let stream = ConnectorStream {
        receiver: event_receiver,
        outcome: outcome_receiver,
        cancellation,
        worker: Some(worker),
        closed: false,
    };

    drop(stream);

    assert!(finished.load(Ordering::Acquire));
}
