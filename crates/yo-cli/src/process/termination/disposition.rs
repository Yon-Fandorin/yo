use std::process;

use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

use super::{PROCESS_STATE, state::Publication};

#[allow(unsafe_code)]
pub(super) fn install(signal: Signal) -> nix::Result<SigAction> {
    let action = SigAction::new(
        SigHandler::Handler(handle_signal),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    // SAFETY: handle_signal performs only lock-free atomic operations and
    // signal-hook's documented async-signal-safe default emulation.
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
    if PROCESS_STATE.publish(signal) == Publication::DefaultNow {
        default_now(signal);
    }
}

pub(super) fn default_now(signal: i32) -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(signal);
    process::abort()
}
