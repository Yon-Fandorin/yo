use std::{
    sync::{Arc, Barrier},
    task::{Context, Poll, Waker},
    thread,
};

use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

use super::super::{
    SIGNALS, TerminationEvents,
    readiness::TerminationReadiness,
    state::{Phase, Publication, SharedState},
};

fn active_shared() -> &'static SharedState {
    let shared = super::support::shared();
    shared
        .transition_preserving(Phase::Installing, Phase::Idle)
        .unwrap();
    shared
        .transition_preserving(Phase::Idle, Phase::Active)
        .unwrap();
    shared
}

// 여러 signal이 cleanup 전 도착하면 고정 우선순위의 원래 signal 하나를 선택한다.
#[test]
fn finalization_selects_the_stable_signal_priority() {
    let shared = active_shared();
    assert_eq!(shared.publish(SIGTERM), Publication::Published);
    assert_eq!(shared.publish(SIGQUIT), Publication::Published);
    assert_eq!(shared.publish(SIGINT), Publication::Published);
    assert_eq!(shared.publish(SIGHUP), Publication::Published);
    shared
        .transition_preserving(Phase::Active, Phase::Cleaning)
        .unwrap();

    assert_eq!(shared.finalize_cleaning().unwrap(), Some(SIGHUP));
    assert_eq!(shared.snapshot_phase(), Phase::Terminating);
}

// ACTIVE에서 CLEANING으로 바뀔 때 이미 게시된 pending bit를 잃지 않는다.
#[test]
fn active_to_cleaning_preserves_pending_bits() {
    let shared = active_shared();
    assert_eq!(shared.publish(SIGINT), Publication::Published);

    shared
        .transition_preserving(Phase::Active, Phase::Cleaning)
        .unwrap();

    let (phase, pending) = shared.snapshot();
    assert_eq!(phase, Phase::Cleaning);
    assert_ne!(pending, 0);
    assert_eq!(shared.finalize_cleaning().unwrap(), Some(SIGINT));
}

// signal 게시와 ACTIVE→CLEANING 전환이 동시에 일어나도 pending bit를 잃지 않는다.
#[test]
fn active_to_cleaning_race_preserves_every_published_signal() {
    for _ in 0..256 {
        let shared = active_shared();
        let gate = Arc::new(Barrier::new(2));
        let worker_gate = Arc::clone(&gate);
        let publisher = thread::spawn(move || {
            worker_gate.wait();
            shared.publish(SIGTERM)
        });

        gate.wait();
        shared
            .transition_preserving(Phase::Active, Phase::Cleaning)
            .unwrap();

        assert_eq!(publisher.join().unwrap(), Publication::Published);
        assert_eq!(shared.finalize_cleaning().unwrap(), Some(SIGTERM));
    }
}

// 여러 thread가 동시에 게시해도 finalization은 완성된 bit 집합에서 고정 우선순위를 고른다.
#[test]
fn concurrent_publication_keeps_stable_signal_priority() {
    let shared = active_shared();
    let gate = Arc::new(Barrier::new(SIGNALS.len() + 1));
    let publishers: Vec<_> = [SIGTERM, SIGQUIT, SIGINT, SIGHUP]
        .into_iter()
        .map(|signal| {
            let worker_gate = Arc::clone(&gate);
            thread::spawn(move || {
                worker_gate.wait();
                shared.publish(signal)
            })
        })
        .collect();

    gate.wait();
    for publisher in publishers {
        assert_eq!(publisher.join().unwrap(), Publication::Published);
    }
    shared
        .transition_preserving(Phase::Active, Phase::Cleaning)
        .unwrap();

    assert_eq!(shared.finalize_cleaning().unwrap(), Some(SIGHUP));
}

// finalization CAS 뒤 IDLE을 본 늦은 signal은 이전 session에 합류하지 않는다.
#[test]
fn signal_after_the_finalization_cutoff_defaults_immediately() {
    let shared = active_shared();
    shared
        .transition_preserving(Phase::Active, Phase::Cleaning)
        .unwrap();
    assert_eq!(shared.finalize_cleaning().unwrap(), None);

    assert_eq!(shared.publish(SIGTERM), Publication::DefaultNow);
    assert_eq!(shared.snapshot_phase(), Phase::Idle);
}

// 게시 phase가 아닌 모든 handler-visible phase는 signal을 보관하지 않고 즉시 default로 보낸다.
#[test]
fn non_publishing_phases_choose_the_default_path() {
    for phase in [
        Phase::Installing,
        Phase::Idle,
        Phase::Terminating,
        Phase::ShuttingDown,
        Phase::Retired,
        Phase::FailedRetired,
    ] {
        let shared = super::support::shared();
        shared.force_phase(phase);

        assert_eq!(shared.publish(SIGTERM), Publication::DefaultNow);
        assert_eq!(shared.snapshot(), (phase, 0));
    }
}

// CLEANING publication과 finalization CAS의 경주는 게시 성공 또는 즉시 default 중 하나다.
#[test]
fn cleaning_finalization_race_never_strands_a_published_signal() {
    for _ in 0..256 {
        let shared = active_shared();
        shared
            .transition_preserving(Phase::Active, Phase::Cleaning)
            .unwrap();
        let gate = Arc::new(Barrier::new(2));
        let worker_gate = Arc::clone(&gate);
        let publisher = thread::spawn(move || {
            worker_gate.wait();
            shared.publish(SIGTERM)
        });

        gate.wait();
        let selected = shared.finalize_cleaning().unwrap();
        let publication = publisher.join().unwrap();

        assert!(matches!(
            (publication, selected),
            (Publication::Published, Some(SIGTERM)) | (Publication::DefaultNow, None)
        ));
    }
}

// ACTIVE Drop의 fail-retire CAS와 signal 게시 경주도 게시 또는 즉시 default 중 하나로 닫힌다.
#[test]
fn fail_retire_race_never_discards_a_published_signal() {
    for _ in 0..256 {
        let shared = active_shared();
        let gate = Arc::new(Barrier::new(2));
        let worker_gate = Arc::clone(&gate);
        let publisher = thread::spawn(move || {
            worker_gate.wait();
            shared.publish(SIGTERM)
        });

        gate.wait();
        let selected = shared.fail_retire();
        let publication = publisher.join().unwrap();

        assert!(matches!(
            (publication, selected),
            (Publication::Published, Some(SIGTERM)) | (Publication::DefaultNow, None)
        ));
        assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
    }
}

// typed event source는 OS signal identity 없이 현재 session의 pending 여부만 투영한다.
#[test]
fn typed_termination_events_hide_signal_identity() {
    let shared = active_shared();
    let mut events = TerminationEvents {
        shared,
        readiness: Arc::new(TerminationReadiness::new()),
    };
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert_eq!(
        yo_tui::TerminationSource::poll_termination(&mut events, &mut context),
        Poll::Pending
    );
    assert_eq!(shared.publish(SIGQUIT), Publication::Published);
    assert_eq!(
        yo_tui::TerminationSource::poll_termination(&mut events, &mut context),
        Poll::Ready(yo_tui::TerminationEvent::Requested)
    );
}
