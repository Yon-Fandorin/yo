use std::process;

use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

use super::{PROCESS_STATE, readiness::signal_wake_fd, state::Publication};

#[allow(unsafe_code)]
pub(super) fn install(signal: Signal) -> nix::Result<SigAction> {
    let action = SigAction::new(
        SigHandler::Handler(handle_signal),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    // SAFETY: handle_signal performs lock-free atomics, an async-signal-safe
    // nonblocking write, and signal-hook's documented default emulation.
    unsafe { sigaction(signal, &action) }
}

#[allow(unsafe_code)]
pub(super) fn restore(signal: Signal, action: &SigAction) -> nix::Result<()> {
    // SAFETY: action is the exact value returned by Nix for this same signal
    // during installation and remains owned by the coordinator.
    unsafe { sigaction(signal, action) }.map(drop)
}

#[allow(unsafe_code)]
pub(in crate::process) fn replace(signal: Signal, action: &SigAction) -> nix::Result<SigAction> {
    // SAFETY: callers provide a Nix-owned action whose handler storage is
    // static. The returned prior action is retained until it is restored.
    unsafe { sigaction(signal, action) }
}

#[cfg(test)]
#[allow(unsafe_code)]
pub(super) fn replace_for_test(signal: Signal, action: &SigAction) -> nix::Result<SigAction> {
    // SAFETY: subprocess tests construct Nix actions with static handlers and
    // keep every action alive until the isolated process exits.
    unsafe { sigaction(signal, action) }
}

extern "C" fn handle_signal(signal: i32) {
    match PROCESS_STATE.publish(signal) {
        Publication::Published => wake_frontend(),
        Publication::DefaultNow => default_now(signal),
    }
}

#[allow(unsafe_code)]
fn wake_frontend() {
    let Some(fd) = signal_wake_fd() else {
        return;
    };
    let byte = 1_u8;
    // SAFETY: fd names the installed nonblocking UnixStream endpoint and the
    // one-byte buffer remains valid for the duration of async-signal-safe write.
    let _ = unsafe { libc::write(fd, std::ptr::from_ref(&byte).cast(), 1) };
}

#[cfg(test)]
pub(super) fn wake_frontend_for_test() {
    wake_frontend();
}

pub(super) fn default_now(signal: i32) -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(signal);
    process::abort()
}
