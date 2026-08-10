use std::panic::{AssertUnwindSafe, catch_unwind};

use nix::sys::signal::Signal;

use super::{
    super::{SIGNALS, TerminationCoordinator, state::Phase},
    support::{Call, coordinator},
};

// 정상 shutdown은 disposition을 역순으로 모두 복구한 뒤 설치 thread mask를 복구한다.
#[test]
fn shutdown_restores_actions_in_reverse_order_and_mask_last() {
    let (mut coordinator, calls) = coordinator([]);

    coordinator.shutdown().unwrap();

    assert_eq!(
        calls.borrow().as_slice(),
        [
            Call::CaptureAndBlock,
            Call::Install(Signal::SIGHUP),
            Call::Install(Signal::SIGINT),
            Call::Install(Signal::SIGQUIT),
            Call::Install(Signal::SIGTERM),
            Call::Unblock,
            Call::Block,
            Call::Restore(Signal::SIGTERM),
            Call::Restore(Signal::SIGQUIT),
            Call::Restore(Signal::SIGINT),
            Call::Restore(Signal::SIGHUP),
            Call::RestoreMask,
        ]
    );
}

// 하나의 coordinator는 session별 재설치 없이 순차 session을 반복해서 빌려준다.
#[test]
fn coordinator_lends_repeated_sessions_without_reinstallation() {
    let (mut coordinator, calls) = coordinator([]);

    assert_eq!(
        coordinator
            .with_active_session(|_| Ok::<_, String>("first"))
            .unwrap(),
        Ok("first")
    );
    assert_eq!(
        coordinator
            .with_active_session(|_| Ok::<_, String>("second"))
            .unwrap(),
        Ok("second")
    );
    assert_eq!(
        calls
            .borrow()
            .iter()
            .filter(|call| matches!(call, Call::Install(_)))
            .count(),
        SIGNALS.len()
    );
    coordinator.shutdown().unwrap();
}

// signal이 선택되지 않은 정상 세대에서는 종료 전용 resource cleanup을 실행하지 않아 다음
// terminal 세대가 같은 application session을 계속 소유할 수 있다.
#[test]
fn normal_generation_keeps_the_shared_resource_alive() {
    let (mut coordinator, _) = coordinator([]);
    let mut resource = "live";
    let mut cleanup_called = false;

    assert_eq!(
        coordinator
            .with_active_resource(
                &mut resource,
                |_, resource| {
                    assert_eq!(*resource, "live");
                    Ok::<_, String>("suspend")
                },
                |_| {
                    cleanup_called = true;
                    Ok::<_, String>(())
                },
            )
            .unwrap(),
        Ok("suspend")
    );
    assert_eq!(resource, "live");
    assert!(!cleanup_called);
    coordinator.shutdown().unwrap();
}

// signal 없는 session panic은 coordinator를 IDLE로 복구한 뒤 원본 payload를 재개한다.
#[test]
fn panic_without_signal_resumes_after_session_finalization() {
    let (mut coordinator, _) = coordinator([]);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _: Result<(), String> = coordinator
            .with_active_session(|_| -> Result<(), String> { panic!("session panic") })
            .unwrap();
    }));

    let payload = result.expect_err("session panic must resume");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"session panic"));
    assert_eq!(coordinator.shared.snapshot_phase(), Phase::Idle);
    coordinator.shutdown().unwrap();
}

// 구성 signal 목록은 SOT가 정한 네 종료 signal과 정확히 일치한다.
#[test]
fn configured_signal_set_is_closed() {
    assert_eq!(
        SIGNALS,
        [
            Signal::SIGHUP,
            Signal::SIGINT,
            Signal::SIGQUIT,
            Signal::SIGTERM,
        ]
    );
}

trait AmbiguousIfSend<A> {
    fn check() {}
}

impl<T: ?Sized> AmbiguousIfSend<()> for T {}

struct IsSend;

impl<T: ?Sized + Send> AmbiguousIfSend<IsSend> for T {}

// Rc 기반 내부 상태 때문에 coordinator가 다른 스레드로 이동할 수 없음을 컴파일 수준에서 확인한다.
#[test]
fn coordinator_type_is_not_send() {
    let _ = <TerminationCoordinator as AmbiguousIfSend<_>>::check;
}
