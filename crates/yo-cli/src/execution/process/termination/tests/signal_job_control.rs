use std::{
    io::Write,
    os::unix::process::ExitStatusExt,
    process::{Command, Stdio},
    sync::{Arc, mpsc},
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};

use nix::sys::signal::{
    SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, pthread_sigmask,
};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

use super::super::{
    PROCESS_STATE, SignalOs, TerminationCoordinator, UnixSignalOs, disposition,
    state::{Phase, Publication},
};

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

const CHILD_ENV: &str = "YO_TERMINATION_SUBPROCESS";

fn run_child(test_name: &str) -> std::process::Output {
    let executable = std::env::current_exe().unwrap();
    let mut child = Command::new("/bin/sh")
        .args(["-c", "ulimit -c 0; exec \"$@\"", "yo-test"])
        .arg(executable)
        .args(["--exact", test_name, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().unwrap() {
            Some(_) => return child.wait_with_output().unwrap(),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            None => {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "termination subprocess timed out: {test_name}\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            },
        }
    }
}

fn assert_child() {
    assert_eq!(std::env::var(CHILD_ENV).as_deref(), Ok("1"));
}

// 실제 SIGINT에서도 active session cleanup 출력이 프로세스 signal 종료보다 먼저 끝난다.
#[test]
fn subprocess_signal_waits_for_active_cleanup() {
    let output = run_child(
        "execution::process::termination::tests::signal_job_control::subprocess_child_signal_waits_for_active_cleanup",
    );
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
            let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
            let mut context = Context::from_waker(&waker);
            while yo_tui::TerminationSource::poll_termination(events, &mut context) == Poll::Pending
            {
                thread::park();
            }
            println!("CLEANUP_DONE");
            std::io::stdout().flush().unwrap();
            Ok(())
        })
        .unwrap();
}

struct ThreadWake(thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

// IDLE은 설치 전 SIG_IGN을 사용하지 않고 coordinator 기본 종료 정책을 적용한다.
#[test]
fn subprocess_idle_overrides_an_ignored_action() {
    let output = run_child(
        "execution::process::termination::tests::signal_job_control::subprocess_child_idle_overrides_an_ignored_action",
    );

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
    let output = run_child(
        "execution::process::termination::tests::signal_job_control::subprocess_child_idle_overrides_a_custom_action",
    );

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
        "execution::process::termination::tests::signal_job_control::subprocess_child_shutdown_restores_action_and_caller_mask",
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
        "execution::process::termination::tests::signal_job_control::subprocess_child_failed_install_restores_action_and_caller_mask",
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
    let output = run_child(
        "execution::process::termination::tests::signal_job_control::subprocess_child_signal_wins_over_session_panic",
    );

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
        "execution::process::termination::tests::signal_job_control::subprocess_child_cleanup_error_precedes_signal_replay",
    );

    assert_eq!(output.status.signal(), Some(SIGTERM));
    assert!(String::from_utf8_lossy(&output.stderr).contains("yo: cleanup failed"));
}

// terminal 세대가 일시정지 결과를 만들었더라도 finalization 중 종료 signal이 선택되면
// 살아 있는 application resource를 먼저 닫고 일시정지 대신 원래 signal로 종료한다.
#[test]
fn subprocess_selected_termination_cleans_shared_resource_before_replay() {
    let output = run_child(
        "execution::process::termination::tests::signal_job_control::subprocess_child_selected_termination_cleans_shared_resource_before_replay",
    );

    assert_eq!(output.status.signal(), Some(SIGTERM));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("RESOURCE_CLEANUP_DONE"),
        "shared resource cleanup must finish before signal replay"
    );
}

// 격리된 child는 operation이 남긴 resource를 termination 전용 callback으로 정리한 뒤
// pending SIGTERM의 기본 동작을 재생한다.
#[test]
#[ignore]
fn subprocess_child_selected_termination_cleans_shared_resource_before_replay() {
    assert_child();
    let mut coordinator = TerminationCoordinator::install().unwrap();
    let shared = coordinator.shared;
    let mut resource = Some("live");

    let _: Result<(), &'static str> = coordinator
        .with_active_resource(
            &mut resource,
            |_, resource| {
                assert_eq!(*resource, Some("live"));
                assert_eq!(shared.publish(SIGTERM), Publication::Published);
                Ok(())
            },
            |resource| {
                assert_eq!(resource.take(), Some("live"));
                println!("RESOURCE_CLEANUP_DONE");
                std::io::stdout().flush().unwrap();
                Ok::<_, &'static str>(())
            },
        )
        .unwrap();
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
