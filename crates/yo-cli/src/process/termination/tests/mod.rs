use std::{
    cell::RefCell,
    collections::BTreeSet,
    io::Write,
    os::unix::process::ExitStatusExt,
    panic::{AssertUnwindSafe, catch_unwind},
    process::{Command, Stdio},
    rc::Rc,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

use nix::sys::signal::{
    SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, pthread_sigmask,
};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

use super::{
    PROCESS_STATE, SIGNALS, SignalOs, TerminationCoordinator, TerminationEvents, UnixSignalOs,
    disposition,
    state::{Phase, Publication, SharedState},
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Call {
    CaptureAndBlock,
    Install(Signal),
    Unblock,
    Block,
    Restore(Signal),
    RestoreMask,
}

#[derive(Clone)]
struct RecordingOs {
    calls: Rc<RefCell<Vec<Call>>>,
    failures: BTreeSet<Call>,
}

impl RecordingOs {
    fn new(failures: impl IntoIterator<Item = Call>) -> (Self, Rc<RefCell<Vec<Call>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                calls: Rc::clone(&calls),
                failures: failures.into_iter().collect(),
            },
            calls,
        )
    }

    fn record(&self, call: Call) -> Result<(), String> {
        self.calls.borrow_mut().push(call.clone());
        if self.failures.contains(&call) {
            Err(format!("{call:?} failed"))
        } else {
            Ok(())
        }
    }
}

impl SignalOs for RecordingOs {
    type Action = Signal;
    type Mask = &'static str;

    fn capture_and_block_configured(&mut self) -> Result<Self::Mask, String> {
        self.record(Call::CaptureAndBlock)?;
        Ok("original mask")
    }

    fn install(&mut self, signal: Signal) -> Result<Self::Action, String> {
        self.record(Call::Install(signal))?;
        Ok(signal)
    }

    fn unblock_configured(&mut self) -> Result<(), String> {
        self.record(Call::Unblock)
    }

    fn block_configured(&mut self) -> Result<(), String> {
        self.record(Call::Block)
    }

    fn restore(&mut self, signal: Signal, _action: &Self::Action) -> Result<(), String> {
        self.record(Call::Restore(signal))
    }

    fn restore_mask(&mut self, _mask: &Self::Mask) -> Result<(), String> {
        self.record(Call::RestoreMask)
    }
}

struct FailingInstallOs {
    inner: UnixSignalOs,
    failed_signal: Signal,
}

impl SignalOs for FailingInstallOs {
    type Action = SigAction;
    type Mask = SigSet;

    fn capture_and_block_configured(&mut self) -> Result<Self::Mask, String> {
        self.inner.capture_and_block_configured()
    }

    fn install(&mut self, signal: Signal) -> Result<Self::Action, String> {
        if signal == self.failed_signal {
            Err(format!("injected {signal:?} installation failure"))
        } else {
            self.inner.install(signal)
        }
    }

    fn unblock_configured(&mut self) -> Result<(), String> {
        self.inner.unblock_configured()
    }

    fn block_configured(&mut self) -> Result<(), String> {
        self.inner.block_configured()
    }

    fn restore(&mut self, signal: Signal, action: &Self::Action) -> Result<(), String> {
        self.inner.restore(signal, action)
    }

    fn restore_mask(&mut self, mask: &Self::Mask) -> Result<(), String> {
        self.inner.restore_mask(mask)
    }
}

fn shared() -> &'static SharedState {
    Box::leak(Box::new(SharedState::installing()))
}

fn coordinator(
    failures: impl IntoIterator<Item = Call>,
) -> (TerminationCoordinator<RecordingOs>, Rc<RefCell<Vec<Call>>>) {
    let shared = shared();
    let (os, calls) = RecordingOs::new(failures);
    let coordinator = TerminationCoordinator::install_with(shared, os, false).unwrap();
    (coordinator, calls)
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
        let shared = shared();
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
    let mut events = TerminationEvents { shared };

    assert_eq!(
        yo_tui::TerminationSource::poll_termination(&mut events),
        yo_tui::TerminationEvent::None
    );
    assert_eq!(shared.publish(SIGQUIT), Publication::Published);
    assert_eq!(
        yo_tui::TerminationSource::poll_termination(&mut events),
        yo_tui::TerminationEvent::Requested
    );
}

fn active_shared() -> &'static SharedState {
    let shared = shared();
    shared
        .transition_preserving(Phase::Installing, Phase::Idle)
        .unwrap();
    shared
        .transition_preserving(Phase::Idle, Phase::Active)
        .unwrap();
    shared
}

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

// 설치 중간 실패는 이미 바뀐 disposition을 역순으로 복구하고 mask 복구도 시도한다.
#[test]
fn install_failure_runs_complete_reverse_rollback() {
    let shared = shared();
    let (os, calls) = RecordingOs::new([
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

// 첫 mask capture 실패도 coordinator를 영구 은퇴시키며 실행하지 않은 복구를 꾸미지 않는다.
#[test]
fn mask_capture_failure_retires_without_spurious_compensation() {
    let shared = shared();
    let (os, calls) = RecordingOs::new([Call::CaptureAndBlock]);

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
        let (os, calls) = RecordingOs::new([Call::Install(failed_signal)]);

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
    let (os, calls) = RecordingOs::new([Call::Unblock]);

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

// Rc phantom ownership makes cross-thread coordinator transfer fail at compile time.
#[test]
fn coordinator_type_is_not_send() {
    let _ = <TerminationCoordinator as AmbiguousIfSend<_>>::check;
}

const CHILD_ENV: &str = "YO_TERMINATION_SUBPROCESS";

fn run_child(test_name: &str) -> std::process::Output {
    let executable = std::env::current_exe().unwrap();
    Command::new("/bin/sh")
        .args(["-c", "ulimit -c 0; exec \"$@\"", "yo-test"])
        .arg(executable)
        .args(["--exact", test_name, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn assert_child() {
    assert_eq!(std::env::var(CHILD_ENV).as_deref(), Ok("1"));
}

// 실제 SIGINT에서도 active session cleanup 출력이 프로세스 signal 종료보다 먼저 끝난다.
#[test]
fn subprocess_signal_waits_for_active_cleanup() {
    let output =
        run_child("process::termination::tests::subprocess_child_signal_waits_for_active_cleanup");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.signal(), Some(SIGINT));
    assert!(stdout.find("SESSION_READY").unwrap() < stdout.find("CLEANUP_DONE").unwrap());
}

// 격리된 child는 active session이 SIGINT를 typed 요청으로 관찰할 때까지 살아 있어야 한다.
// 요청을 받은 뒤 cleanup 완료를 출력하고 나서야 원래 SIGINT 기본 종료를 재생한다.
#[test]
#[ignore]
fn subprocess_child_signal_waits_for_active_cleanup() {
    assert_child();
    let mut coordinator = TerminationCoordinator::install().unwrap();
    let (active_tx, active_rx) = mpsc::sync_channel(0);
    let _sender = thread::spawn(move || {
        active_rx.recv().unwrap();
        signal_hook::low_level::raise(SIGINT).unwrap();
    });

    let _: Result<(), String> = coordinator
        .with_active_session(|events| {
            println!("SESSION_READY");
            std::io::stdout().flush().unwrap();
            active_tx.send(()).unwrap();
            while yo_tui::TerminationSource::poll_termination(events)
                == yo_tui::TerminationEvent::None
            {
                thread::sleep(Duration::from_millis(1));
            }
            println!("CLEANUP_DONE");
            std::io::stdout().flush().unwrap();
            Ok(())
        })
        .unwrap();
}

// IDLE은 설치 전 SIG_IGN을 사용하지 않고 coordinator 기본 종료 정책을 적용한다.
#[test]
fn subprocess_idle_overrides_an_ignored_action() {
    let output =
        run_child("process::termination::tests::subprocess_child_idle_overrides_an_ignored_action");

    assert_eq!(output.status.signal(), Some(SIGTERM));
}

// 격리된 child에 SIGTERM ignore action을 먼저 설치해도 coordinator가 이를 실행 정책으로 물려받지
// 않는다. IDLE에서 SIGTERM을 올리면 뒤의 panic까지 가지 않고 기본 signal 종료가 일어나야 한다.
#[test]
#[ignore]
fn subprocess_child_idle_overrides_an_ignored_action() {
    assert_child();
    let ignored = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
    disposition::replace_for_test(Signal::SIGTERM, &ignored).unwrap();
    let _coordinator = TerminationCoordinator::install().unwrap();

    signal_hook::low_level::raise(SIGTERM).unwrap();
    panic!("idle default replay must terminate the child");
}

extern "C" fn prior_custom_handler(_signal: i32) {}

// IDLE은 설치 전 custom handler도 호출하지 않고 coordinator 기본 정책을 적용한다.
#[test]
fn subprocess_idle_overrides_a_custom_action() {
    let output =
        run_child("process::termination::tests::subprocess_child_idle_overrides_a_custom_action");

    assert_eq!(output.status.signal(), Some(SIGHUP));
}

// 격리된 child에 custom SIGHUP handler가 있어도 coordinator의 IDLE 정책은 그 handler를 호출하지
// 않는다. SIGHUP을 올리면 뒤의 panic까지 가지 않고 기본 signal 종료가 일어나야 한다.
#[test]
#[ignore]
fn subprocess_child_idle_overrides_a_custom_action() {
    assert_child();
    let custom = SigAction::new(
        SigHandler::Handler(prior_custom_handler),
        SaFlags::SA_NODEFER,
        SigSet::empty(),
    );
    disposition::replace_for_test(Signal::SIGHUP, &custom).unwrap();
    let _coordinator = TerminationCoordinator::install().unwrap();

    signal_hook::low_level::raise(SIGHUP).unwrap();
    panic!("idle default replay must replace the custom handler");
}

// 성공한 shutdown 뒤에는 설치 전 ignore action과 caller mask가 정확히 다시 보인다.
#[test]
fn subprocess_shutdown_restores_action_and_caller_mask() {
    let output = run_child(
        "process::termination::tests::subprocess_child_shutdown_restores_action_and_caller_mask",
    );

    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(String::from_utf8_lossy(&output.stdout).contains("RESTORED"));
}

// 격리된 child에서 shutdown한 뒤 설치 전 SIGINT action과 caller signal mask가 정확히 복원된다.
// 은퇴 뒤의 delayed publication 시도도 DefaultNow가 되어 pending bit로 보관되지 않는다.
#[test]
#[ignore]
fn subprocess_child_shutdown_restores_action_and_caller_mask() {
    assert_child();
    let mut action_mask = SigSet::empty();
    action_mask.add(Signal::SIGTERM);
    let prior_action = SigAction::new(
        SigHandler::SigIgn,
        SaFlags::SA_NODEFER | SaFlags::SA_RESETHAND,
        action_mask,
    );
    disposition::replace_for_test(Signal::SIGINT, &prior_action).unwrap();
    let expected_action = disposition::replace_for_test(Signal::SIGINT, &prior_action).unwrap();
    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGTERM);
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&blocked), None).unwrap();
    let mut original_mask = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_SETMASK, None, Some(&mut original_mask)).unwrap();

    let mut coordinator = TerminationCoordinator::install().unwrap();
    coordinator.shutdown().unwrap();

    let mut current = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_SETMASK, None, Some(&mut current)).unwrap();
    assert_eq!(current, original_mask);
    let probe = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    let restored = disposition::replace_for_test(Signal::SIGINT, &probe).unwrap();
    assert!(matches!(restored.handler(), SigHandler::SigIgn));
    assert_eq!(restored.flags(), expected_action.flags());
    assert_eq!(restored.mask(), expected_action.mask());
    let delayed_publication = thread::spawn(|| PROCESS_STATE.publish(SIGTERM))
        .join()
        .unwrap();
    assert_eq!(delayed_publication, Publication::DefaultNow);
    println!("RESTORED");
}

// 실제 설치 중간 실패도 이미 바꾼 action과 caller mask를 정확히 원상 복구한다.
#[test]
fn subprocess_failed_install_restores_action_and_caller_mask() {
    let output = run_child(
        "process::termination::tests::subprocess_child_failed_install_restores_action_and_caller_mask",
    );

    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(String::from_utf8_lossy(&output.stdout).contains("ROLLBACK_RESTORED"));
}

// 실제 OS action 설치를 SIGINT 단계에서 실패시키면 앞서 바꾼 SIGHUP action과 caller mask를
// 복구한다. 실패 은퇴 뒤의 delayed publication도 DefaultNow가 되어 pending으로 남지 않는다.
#[test]
#[ignore]
fn subprocess_child_failed_install_restores_action_and_caller_mask() {
    assert_child();
    let mut action_mask = SigSet::empty();
    action_mask.add(Signal::SIGTERM);
    let prior_action = SigAction::new(
        SigHandler::SigIgn,
        SaFlags::SA_NODEFER | SaFlags::SA_RESETHAND,
        action_mask,
    );
    disposition::replace_for_test(Signal::SIGHUP, &prior_action).unwrap();
    let expected_action = disposition::replace_for_test(Signal::SIGHUP, &prior_action).unwrap();
    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGTERM);
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&blocked), None).unwrap();
    let mut original_mask = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_SETMASK, None, Some(&mut original_mask)).unwrap();
    PROCESS_STATE
        .transition_preserving(Phase::New, Phase::Installing)
        .unwrap();

    let os = FailingInstallOs {
        inner: UnixSignalOs,
        failed_signal: Signal::SIGINT,
    };
    let error = match TerminationCoordinator::install_with(&PROCESS_STATE, os, false) {
        Ok(_) => panic!("injected install failure must reject initialization"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("injected SIGINT"));

    let mut current_mask = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_SETMASK, None, Some(&mut current_mask)).unwrap();
    assert_eq!(current_mask, original_mask);
    let probe = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    let restored = disposition::replace_for_test(Signal::SIGHUP, &probe).unwrap();
    assert!(matches!(restored.handler(), SigHandler::SigIgn));
    assert_eq!(restored.flags(), expected_action.flags());
    assert_eq!(restored.mask(), expected_action.mask());
    let delayed_publication = thread::spawn(|| PROCESS_STATE.publish(SIGTERM))
        .join()
        .unwrap();
    assert_eq!(delayed_publication, Publication::DefaultNow);
    println!("ROLLBACK_RESTORED");
}

struct CleanupMarker;

impl Drop for CleanupMarker {
    fn drop(&mut self) {
        println!("PANIC_CLEANUP_DONE");
        std::io::stdout().flush().unwrap();
    }
}

// cleanup 중 signal bit가 이미 게시됐다면 concurrent panic보다 같은 signal 종료가 이긴다.
#[test]
fn subprocess_signal_wins_over_session_panic() {
    let output =
        run_child("process::termination::tests::subprocess_child_signal_wins_over_session_panic");

    assert_eq!(output.status.signal(), Some(SIGQUIT));
    assert!(String::from_utf8_lossy(&output.stdout).contains("PANIC_CLEANUP_DONE"));
}

// active session이 SIGQUIT을 게시한 뒤 panic해도 stack cleanup은 먼저 실행된다.
// cleanup이 끝나면 panic을 계속 전파하는 대신 pending SIGQUIT 기본 종료를 재생해야 한다.
#[test]
#[ignore]
fn subprocess_child_signal_wins_over_session_panic() {
    assert_child();
    let mut coordinator = TerminationCoordinator::install().unwrap();
    let shared = coordinator.shared;

    let _: Result<(), String> = coordinator
        .with_active_session(|_| -> Result<(), String> {
            let _cleanup = CleanupMarker;
            assert_eq!(shared.publish(SIGQUIT), Publication::Published);
            panic!("signal must win over this panic");
        })
        .unwrap();
}

// cleanup 오류와 signal이 겹치면 오류를 먼저 출력하고 같은 signal로 종료한다.
#[test]
fn subprocess_cleanup_error_precedes_signal_replay() {
    let output = run_child(
        "process::termination::tests::subprocess_child_cleanup_error_precedes_signal_replay",
    );

    assert_eq!(output.status.signal(), Some(SIGTERM));
    assert!(String::from_utf8_lossy(&output.stderr).contains("yo: cleanup failed"));
}

// active session이 SIGTERM을 게시하고 cleanup 오류를 반환하면 coordinator가 오류를 stderr에
// 먼저 출력한 뒤 pending SIGTERM 기본 종료를 재생한다.
#[test]
#[ignore]
fn subprocess_child_cleanup_error_precedes_signal_replay() {
    assert_child();
    let mut coordinator = TerminationCoordinator::install().unwrap();
    let shared = coordinator.shared;

    let _: Result<(), &'static str> = coordinator
        .with_active_session(|_| {
            assert_eq!(shared.publish(SIGTERM), Publication::Published);
            Err("cleanup failed")
        })
        .unwrap();
}
