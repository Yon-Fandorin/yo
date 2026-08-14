use std::{
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};

pub(super) fn reap_child(
    child: &mut Child,
    leader_exited: bool,
    cleanup_grace: Duration,
) -> (Option<ExitStatus>, bool) {
    if leader_exited {
        return child
            .wait()
            .map_or((None, true), |status| (Some(status), false));
    }
    let _ = child.kill();
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (Some(status), false),
            Ok(None) if started.elapsed() < cleanup_grace => {
                thread::sleep(Duration::from_millis(10));
            },
            Ok(None) | Err(_) => return (None, true),
        }
    }
}

pub(super) fn terminate_process_group(child: &mut Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        terminate_process_group_id(Pid::from_raw(pid));
    }
    let _ = child.kill();
}

pub(super) fn terminate_process_group_id(process_group: Pid) {
    let _ = kill(Pid::from_raw(-process_group.as_raw()), Signal::SIGKILL);
}

// WNOWAIT leaves the exited group leader as a zombie, so its numeric process-group ID
// remains pinned until the caller has killed any descendants and explicitly reaped it.
#[allow(unsafe_code)]
pub(super) fn child_exited_without_reaping(child: Pid) -> bool {
    // SAFETY: waitid initializes siginfo_t on success. The zeroed value also makes the
    // WNOHANG/no-state-change result distinguishable by its zero si_pid on Linux and macOS.
    let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.as_raw() as libc::id_t,
            &mut information,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    result == 0 && unsafe { information.si_pid() } != 0
}
