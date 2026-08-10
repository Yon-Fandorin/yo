use std::{
    sync::{Arc, mpsc},
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use nix::sys::signal::Signal;

use super::{
    super::{
        SIGNALS, TerminationCoordinator, disposition,
        readiness::{TerminationNotifier, TerminationReadiness, signal_wake_fd},
        state::Phase,
    },
    support::{Call, coordinator, shared},
};

// 설치 중간 실패는 이미 바뀐 disposition을 역순으로 복구하고 mask 복구도 시도한다.
#[test]
fn install_failure_runs_complete_reverse_rollback() {
    let shared = shared();
    let (os, calls) = super::support::RecordingOs::new([
        Call::Install(Signal::SIGQUIT),
        Call::Restore(Signal::SIGINT),
        Call::RestoreMask,
    ]);

    let error = match TerminationCoordinator::install_with(shared, os, false) {
        Ok(_) => panic!("injected install failure must reject initialization"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("Install(SIGQUIT) failed"));
    assert!(error.to_string().contains("Restore(SIGINT) failed"));
    assert!(error.to_string().contains("RestoreMask failed"));
    assert_eq!(
        calls.borrow().as_slice(),
        [
            Call::CaptureAndBlock,
            Call::Install(Signal::SIGHUP),
            Call::Install(Signal::SIGINT),
            Call::Install(Signal::SIGQUIT),
            Call::Restore(Signal::SIGINT),
            Call::Restore(Signal::SIGHUP),
            Call::RestoreMask,
        ]
    );
    assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
}

struct ChannelWake(mpsc::Sender<()>);

impl Wake for ChannelWake {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.0.send(());
    }
}

// 실제 notifier는 FD를 게시하고 relay로 frontend를 깨운 뒤 후속 설치 실패 시 FD와 thread를
// 회수합니다.
#[test]
fn notifier_lifecycle_rolls_back_with_a_later_install_failure() {
    let shared = shared();
    let readiness = Arc::new(TerminationReadiness::new());
    let notifier = TerminationNotifier::install(Arc::clone(&readiness)).unwrap();
    assert!(signal_wake_fd().is_some());

    let (wake_tx, wake_rx) = mpsc::channel();
    let waker = Waker::from(Arc::new(ChannelWake(wake_tx)));
    let mut context = Context::from_waker(&waker);
    assert_eq!(readiness.poll(&mut context), Poll::Pending);
    disposition::wake_frontend_for_test();
    wake_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(readiness.poll(&mut context), Poll::Ready(()));

    let (os, _) = super::support::RecordingOs::new([Call::Install(Signal::SIGQUIT)]);
    let error = match TerminationCoordinator::install_with_parts(
        shared,
        os,
        false,
        readiness,
        Some(notifier),
    ) {
        Ok(_) => panic!("injected install failure must reject initialization"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("Install(SIGQUIT) failed"));
    assert_eq!(signal_wake_fd(), None);
}

// 첫 mask capture 실패도 coordinator를 영구 은퇴시키며 실행하지 않은 복구를 꾸미지 않는다.
#[test]
fn mask_capture_failure_retires_without_spurious_compensation() {
    let shared = shared();
    let (os, calls) = super::support::RecordingOs::new([Call::CaptureAndBlock]);

    let error = match TerminationCoordinator::install_with(shared, os, false) {
        Ok(_) => panic!("injected mask capture failure must reject initialization"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("CaptureAndBlock failed"));
    assert_eq!(calls.borrow().as_slice(), [Call::CaptureAndBlock]);
    assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
}

// 어느 disposition 설치 단계가 실패해도 그 앞 단계만 정확히 역순 복구한다.
#[test]
fn every_handler_installation_step_has_a_complete_rollback() {
    for (failed_index, failed_signal) in SIGNALS.into_iter().enumerate() {
        let shared = shared();
        let (os, calls) = super::support::RecordingOs::new([Call::Install(failed_signal)]);

        let error = match TerminationCoordinator::install_with(shared, os, false) {
            Ok(_) => panic!("injected handler installation failure must reject initialization"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("failed"));
        let calls = calls.borrow();
        let restored = calls
            .iter()
            .filter(|call| matches!(call, Call::Restore(_)))
            .count();
        assert_eq!(restored, failed_index);
        assert_eq!(calls.last(), Some(&Call::RestoreMask));
        assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
    }
}

// 마지막 unblock 실패도 설치된 모든 disposition과 원래 mask를 역순으로 복구한다.
#[test]
fn unblock_failure_rolls_back_every_installed_action() {
    let shared = shared();
    let (os, calls) = super::support::RecordingOs::new([Call::Unblock]);

    let error = match TerminationCoordinator::install_with(shared, os, false) {
        Ok(_) => panic!("injected unblock failure must reject initialization"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("Unblock failed"));
    let calls = calls.borrow();
    assert_eq!(
        &calls[calls.len() - 5..],
        [
            Call::Restore(Signal::SIGTERM),
            Call::Restore(Signal::SIGQUIT),
            Call::Restore(Signal::SIGINT),
            Call::Restore(Signal::SIGHUP),
            Call::RestoreMask,
        ]
    );
    assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
}

// shutdown 일부 복구가 실패해도 나머지 disposition과 mask를 모두 시도하고 영구 은퇴한다.
#[test]
fn shutdown_failure_attempts_every_compensation_and_retires() {
    let (mut coordinator, log) = coordinator([
        Call::Block,
        Call::Restore(Signal::SIGQUIT),
        Call::Restore(Signal::SIGHUP),
        Call::RestoreMask,
    ]);

    let error = coordinator.shutdown().unwrap_err();

    assert!(error.to_string().contains("blocking configured"));
    assert!(error.to_string().contains("SIGQUIT"));
    assert!(error.to_string().contains("SIGHUP"));
    assert!(error.to_string().contains("signal mask"));
    let call_count = {
        let calls = log.borrow();
        assert_eq!(
            &calls[calls.len() - 6..],
            [
                Call::Block,
                Call::Restore(Signal::SIGTERM),
                Call::Restore(Signal::SIGQUIT),
                Call::Restore(Signal::SIGINT),
                Call::Restore(Signal::SIGHUP),
                Call::RestoreMask,
            ]
        );
        calls.len()
    };
    assert_eq!(coordinator.shared.snapshot_phase(), Phase::FailedRetired);
    drop(coordinator);
    assert_eq!(log.borrow().len(), call_count);
}

// shutdown의 어느 단계가 단독 실패해도 전체 복구를 시도하고 FAILED_RETIRED로 닫힌다.
#[test]
fn every_shutdown_step_failure_attempts_the_complete_sequence() {
    let failures = [
        Call::Block,
        Call::Restore(Signal::SIGTERM),
        Call::Restore(Signal::SIGQUIT),
        Call::Restore(Signal::SIGINT),
        Call::Restore(Signal::SIGHUP),
        Call::RestoreMask,
    ];
    let expected = [
        Call::Block,
        Call::Restore(Signal::SIGTERM),
        Call::Restore(Signal::SIGQUIT),
        Call::Restore(Signal::SIGINT),
        Call::Restore(Signal::SIGHUP),
        Call::RestoreMask,
    ];

    for failure in failures {
        let (mut coordinator, calls) = coordinator([failure]);
        let shared = coordinator.shared;

        assert!(coordinator.shutdown().is_err());

        let calls = calls.borrow();
        assert_eq!(&calls[calls.len() - expected.len()..], expected);
        assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
    }
}

// IDLE Drop은 report를 버리더라도 명시적 shutdown과 같은 best-effort 복구를 실행한다.
#[test]
fn idle_drop_attempts_the_complete_shutdown_sequence() {
    let (coordinator, calls) = coordinator([Call::Restore(Signal::SIGINT)]);

    drop(coordinator);

    let calls = calls.borrow();
    assert_eq!(
        &calls[calls.len() - 6..],
        [
            Call::Block,
            Call::Restore(Signal::SIGTERM),
            Call::Restore(Signal::SIGQUIT),
            Call::Restore(Signal::SIGINT),
            Call::Restore(Signal::SIGHUP),
            Call::RestoreMask,
        ]
    );
}

// ACTIVE Drop은 disposition을 조기에 복구하지 않고 fail-closed 은퇴 상태만 게시한다.
#[test]
fn active_drop_does_not_restore_process_dispositions() {
    let (coordinator, calls) = coordinator([]);
    coordinator
        .shared
        .transition_preserving(Phase::Idle, Phase::Active)
        .unwrap();
    let shared = coordinator.shared;
    let call_count = calls.borrow().len();

    drop(coordinator);

    assert_eq!(calls.borrow().len(), call_count);
    assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
}

// INSTALLING, CLEANING, TERMINATING과 partial lifecycle Drop은 disposition을 조기에 복구하지
// 않는다.
#[test]
fn non_idle_drop_phases_never_restore_process_dispositions() {
    for phase in [
        Phase::Installing,
        Phase::Active,
        Phase::Cleaning,
        Phase::Terminating,
        Phase::ShuttingDown,
        Phase::FailedRetired,
    ] {
        let (coordinator, calls) = coordinator([]);
        coordinator.shared.force_phase(phase);
        let shared = coordinator.shared;
        let call_count = calls.borrow().len();

        drop(coordinator);

        assert_eq!(calls.borrow().len(), call_count);
        assert_eq!(shared.snapshot_phase(), Phase::FailedRetired);
    }
}

// 성공한 shutdown 뒤 Drop은 RETIRED 상태와 완료된 복구 기록을 그대로 둔다.
#[test]
fn retired_drop_is_idempotent() {
    let (mut coordinator, calls) = coordinator([]);
    let shared = coordinator.shared;
    coordinator.shutdown().unwrap();
    let call_count = calls.borrow().len();

    drop(coordinator);

    assert_eq!(calls.borrow().len(), call_count);
    assert_eq!(shared.snapshot_phase(), Phase::Retired);
}
