use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use super::{
    PanicDiagnostic, PanicHook, PanicLocation, PanicOutcome, PanicRouteError, catch_owner_panic,
};

static TEST_HOOK_OWNER: Mutex<()> = Mutex::new(());

// terminal을 소유한 현재 thread의 첫 panic만 owned metadata로 보관한다.
#[test]
fn owner_thread_first_panic_is_captured_once() {
    let _test_hook_owner = lock_test_hook();
    let outcome = catch_owner_panic(|| {
        let _ = catch_unwind(AssertUnwindSafe(|| panic!("first failure")));
        panic!("second failure");
    })
    .unwrap();

    assert!(outcome.result.is_err());
    assert_eq!(outcome.diagnostic.unwrap().message, "first failure");
}

// 다른 thread의 panic과 router 복구 뒤 panic은 설치 전 custom hook에 도달한다.
#[test]
fn previous_hook_receives_other_thread_and_post_restore_panics() {
    let _test_hook_owner = lock_test_hook();
    let original: Arc<PanicHook> = std::panic::take_hook().into();
    let worker_calls = Arc::new(AtomicUsize::new(0));
    let restored_calls = Arc::new(AtomicUsize::new(0));
    let hook_worker_calls = Arc::clone(&worker_calls);
    let hook_restored_calls = Arc::clone(&restored_calls);
    let hook_original = Arc::clone(&original);
    std::panic::set_hook(Box::new(move |info| {
        match info.payload().downcast_ref::<&str>() {
            Some(&"worker failure") => {
                hook_worker_calls.fetch_add(1, Ordering::Relaxed);
            },
            Some(&"after restore") => {
                hook_restored_calls.fetch_add(1, Ordering::Relaxed);
            },
            _ => {},
        }
        hook_original(info);
    }));

    let outcome = catch_owner_panic(|| {
        thread::spawn(|| {
            let _ = catch_unwind(AssertUnwindSafe(|| panic!("worker failure")));
        })
        .join()
        .unwrap();
    })
    .unwrap();
    let _ = catch_unwind(AssertUnwindSafe(|| panic!("after restore")));
    std::panic::set_hook(Box::new(move |info| original(info)));

    assert!(outcome.result.is_ok());
    assert!(outcome.diagnostic.is_none());
    assert_eq!(worker_calls.load(Ordering::Relaxed), 1);
    assert_eq!(restored_calls.load(Ordering::Relaxed), 1);
}

// 내부 catch 뒤 cleanup을 끝내고 재개한 원래 panic을 바깥 router가 복구 후 반환한다.
#[test]
fn complete_terminal_boundary_cleans_up_before_returning_the_panic() {
    let _test_hook_owner = lock_test_hook();
    let cleaned = Arc::new(AtomicUsize::new(0));
    let cleanup_observer = Arc::clone(&cleaned);
    let outcome = catch_owner_panic(AssertUnwindSafe(|| {
        let result: Result<(), _> = catch_unwind(AssertUnwindSafe(|| panic!("operation failure")));
        cleanup_observer.fetch_add(1, Ordering::Relaxed);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }))
    .unwrap();

    assert!(outcome.result.is_err());
    assert_eq!(cleaned.load(Ordering::Relaxed), 1);
    assert_eq!(outcome.diagnostic.unwrap().message, "operation failure");
}

// 중첩 ownership은 기다리며 교착하지 않고 즉시 구조화된 오류로 거절한다.
#[test]
fn overlapping_install_is_rejected_without_blocking() {
    let _test_hook_owner = lock_test_hook();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let worker = thread::spawn(move || {
        catch_owner_panic(|| {
            worker_entered.wait();
            worker_release.wait();
        })
        .unwrap()
    });
    entered.wait();

    let overlapping = catch_owner_panic(|| ());

    assert!(matches!(
        overlapping,
        Err(PanicRouteError::AlreadyInstalled)
    ));
    release.wait();
    let PanicOutcome { result, diagnostic } = worker.join().unwrap();
    assert!(result.is_ok());
    assert!(diagnostic.is_none());
}

// 보관한 진단은 hook 내부 참조 없이 복구 후 안정적으로 출력할 수 있다.
#[test]
fn retained_diagnostic_has_a_stable_owned_projection() {
    let _test_hook_owner = lock_test_hook();
    let diagnostic = PanicDiagnostic {
        thread: "main".to_owned(),
        message: "unexpected state".to_owned(),
        location: Some(PanicLocation {
            file: "src/main.rs".to_owned(),
            line: 12,
            column: 7,
        }),
    };
    let mut output = Vec::new();

    diagnostic.emit(&mut output).unwrap();

    assert_eq!(
        output,
        b"thread 'main' panicked at src/main.rs:12:7:\nunexpected state\n"
    );
}

fn lock_test_hook() -> std::sync::MutexGuard<'static, ()> {
    TEST_HOOK_OWNER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
