use std::{
    error::Error,
    fmt,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::atomic::{AtomicU8, Ordering},
    thread::{self, ThreadId},
};

use nix::sys::signal::{SigAction, SigSet, SigmaskHow, Signal, pthread_sigmask};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

use self::state::{Phase, SharedState, StateError};

mod disposition;
mod state;

static PROCESS_COORDINATOR: AtomicU8 = AtomicU8::new(0);
static PROCESS_STATE: SharedState = SharedState::new();
const SIGNALS: [Signal; 4] = [
    Signal::SIGHUP,
    Signal::SIGINT,
    Signal::SIGQUIT,
    Signal::SIGTERM,
];

pub(crate) struct TerminationCoordinator<O = UnixSignalOs>
where
    O: SignalOs,
{
    shared: &'static SharedState,
    os: O,
    prior_actions: Vec<(Signal, O::Action)>,
    original_mask: O::Mask,
    owner: ThreadId,
    retired: bool,
    owns_process_claim: bool,
    _thread_bound: PhantomData<Rc<()>>,
}

impl TerminationCoordinator<UnixSignalOs> {
    pub(crate) fn install() -> Result<Self, HostError> {
        if PROCESS_COORDINATOR
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(HostError::single(
                "installing process termination coordinator",
                "the irreversible process coordinator was already initialized",
            ));
        }

        if let Err(error) = PROCESS_STATE.transition_preserving(Phase::New, Phase::Installing) {
            PROCESS_COORDINATOR.store(2, Ordering::SeqCst);
            return Err(HostError::state(error));
        }
        match Self::install_with(&PROCESS_STATE, UnixSignalOs, true) {
            Ok(coordinator) => Ok(coordinator),
            Err(error) => {
                PROCESS_COORDINATOR.store(2, Ordering::SeqCst);
                Err(error)
            },
        }
    }
}

impl<O> TerminationCoordinator<O>
where
    O: SignalOs,
{
    fn install_with(
        shared: &'static SharedState,
        mut os: O,
        owns_process_claim: bool,
    ) -> Result<Self, HostError> {
        let original_mask = os.capture_and_block_configured().map_err(|error| {
            shared.force_phase(Phase::FailedRetired);
            HostError::single("capturing the installing thread signal mask", error)
        })?;
        let mut prior_actions = Vec::with_capacity(SIGNALS.len());

        for signal in SIGNALS {
            match os.install(signal) {
                Ok(previous) => prior_actions.push((signal, previous)),
                Err(primary) => {
                    let failures =
                        rollback_install(&mut os, &prior_actions, &original_mask, primary);
                    shared.force_phase(Phase::FailedRetired);
                    return Err(HostError::many(
                        "installing process termination dispositions",
                        failures,
                    ));
                },
            }
        }

        if let Err(primary) = os.unblock_configured() {
            let failures = rollback_install(&mut os, &prior_actions, &original_mask, primary);
            shared.force_phase(Phase::FailedRetired);
            return Err(HostError::many(
                "unblocking configured termination signals",
                failures,
            ));
        }

        shared
            .transition_preserving(Phase::Installing, Phase::Idle)
            .map_err(HostError::state)?;
        Ok(Self {
            shared,
            os,
            prior_actions,
            original_mask,
            owner: thread::current().id(),
            retired: false,
            owns_process_claim,
            _thread_bound: PhantomData,
        })
    }

    pub(crate) fn with_active_session<T, E>(
        &mut self,
        operation: impl FnOnce(&mut TerminationEvents) -> Result<T, E>,
    ) -> Result<Result<T, E>, HostError>
    where
        E: fmt::Display,
    {
        self.require_owner()?;
        if self.retired {
            return Err(HostError::single(
                "starting an active terminal session",
                "the process coordinator is retired",
            ));
        }
        self.shared
            .transition_preserving(Phase::Idle, Phase::Active)
            .map_err(HostError::state)?;

        let mut events = TerminationEvents {
            shared: self.shared,
        };
        let result = catch_unwind(AssertUnwindSafe(|| operation(&mut events)));
        self.shared
            .transition_preserving(Phase::Active, Phase::Cleaning)
            .map_err(HostError::state)?;
        let selected = self.shared.finalize_cleaning().map_err(HostError::state)?;

        if let Some(signal) = selected {
            if let Ok(Err(error)) = &result {
                use std::io::Write;
                let _ = writeln!(std::io::stderr().lock(), "yo: {error}");
            }
            disposition::default_now(signal);
        }

        match result {
            Ok(value) => Ok(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), HostError> {
        self.require_owner()?;
        if self.retired {
            return Err(HostError::single(
                "shutting down process termination coordinator",
                "the process coordinator is already retired",
            ));
        }
        self.shared
            .transition_preserving(Phase::Idle, Phase::ShuttingDown)
            .map_err(HostError::state)?;

        let mut failures = Vec::new();
        if let Err(error) = self.os.block_configured() {
            failures.push(format!("blocking configured termination signals: {error}"));
        }
        for (signal, action) in self.prior_actions.iter().rev() {
            if let Err(error) = self.os.restore(*signal, action) {
                failures.push(format!("restoring {signal:?}: {error}"));
            }
        }
        if let Err(error) = self.os.restore_mask(&self.original_mask) {
            failures.push(format!(
                "restoring the installing thread signal mask: {error}"
            ));
        }

        self.retired = true;
        if self.owns_process_claim {
            PROCESS_COORDINATOR.store(2, Ordering::SeqCst);
        }
        if failures.is_empty() {
            self.shared.force_phase(Phase::Retired);
            Ok(())
        } else {
            self.shared.force_phase(Phase::FailedRetired);
            Err(HostError::many(
                "shutting down process termination coordinator",
                failures,
            ))
        }
    }

    fn require_owner(&self) -> Result<(), HostError> {
        if thread::current().id() == self.owner {
            Ok(())
        } else {
            Err(HostError::single(
                "using process termination coordinator",
                "operation ran on a thread other than the installing thread",
            ))
        }
    }
}

impl<O> Drop for TerminationCoordinator<O>
where
    O: SignalOs,
{
    fn drop(&mut self) {
        if self.retired || thread::current().id() != self.owner {
            return;
        }
        let phase = self.shared.snapshot_phase();
        if phase == Phase::Idle {
            let _ = self.shutdown();
        } else {
            let selected = self.shared.fail_retire();
            self.retired = true;
            if self.owns_process_claim {
                PROCESS_COORDINATOR.store(2, Ordering::SeqCst);
            }
            if let Some(signal) = selected {
                disposition::default_now(signal);
            }
        }
    }
}

pub(crate) struct TerminationEvents {
    shared: &'static SharedState,
}

impl yo_tui::TerminationSource for TerminationEvents {
    fn poll_termination(&mut self) -> yo_tui::TerminationEvent {
        if self.shared.observe_termination() {
            yo_tui::TerminationEvent::Requested
        } else {
            yo_tui::TerminationEvent::None
        }
    }
}

pub(crate) trait SignalOs {
    type Action;
    type Mask;

    fn capture_and_block_configured(&mut self) -> Result<Self::Mask, String>;
    fn install(&mut self, signal: Signal) -> Result<Self::Action, String>;
    fn unblock_configured(&mut self) -> Result<(), String>;
    fn block_configured(&mut self) -> Result<(), String>;
    fn restore(&mut self, signal: Signal, action: &Self::Action) -> Result<(), String>;
    fn restore_mask(&mut self, mask: &Self::Mask) -> Result<(), String>;
}

pub(crate) struct UnixSignalOs;

impl SignalOs for UnixSignalOs {
    type Action = SigAction;
    type Mask = SigSet;

    fn capture_and_block_configured(&mut self) -> Result<Self::Mask, String> {
        let signals = configured_signal_set();
        let mut mask = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&signals), Some(&mut mask))
            .map_err(|error| error.to_string())?;
        Ok(mask)
    }

    fn install(&mut self, signal: Signal) -> Result<Self::Action, String> {
        disposition::install(signal).map_err(|error| error.to_string())
    }

    fn unblock_configured(&mut self) -> Result<(), String> {
        let signals = configured_signal_set();
        pthread_sigmask(SigmaskHow::SIG_UNBLOCK, Some(&signals), None)
            .map_err(|error| error.to_string())
    }

    fn block_configured(&mut self) -> Result<(), String> {
        let signals = configured_signal_set();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&signals), None)
            .map_err(|error| error.to_string())
    }

    fn restore(&mut self, signal: Signal, action: &Self::Action) -> Result<(), String> {
        disposition::restore(signal, action).map_err(|error| error.to_string())
    }

    fn restore_mask(&mut self, mask: &Self::Mask) -> Result<(), String> {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(mask), None)
            .map_err(|error| error.to_string())
    }
}

fn configured_signal_set() -> SigSet {
    let mut signals = SigSet::empty();
    for signal in SIGNALS {
        signals.add(signal);
    }
    signals
}

fn rollback_install<O>(
    os: &mut O,
    prior_actions: &[(Signal, O::Action)],
    original_mask: &O::Mask,
    primary: String,
) -> Vec<String>
where
    O: SignalOs,
{
    let mut failures = vec![primary];
    for (signal, action) in prior_actions.iter().rev() {
        if let Err(error) = os.restore(*signal, action) {
            failures.push(format!("rolling back {signal:?}: {error}"));
        }
    }
    if let Err(error) = os.restore_mask(original_mask) {
        failures.push(format!(
            "rolling back the installing thread signal mask: {error}"
        ));
    }
    failures
}

#[derive(Debug)]
pub(crate) struct HostError {
    context: &'static str,
    failures: Vec<String>,
}

impl HostError {
    fn single(context: &'static str, failure: impl Into<String>) -> Self {
        Self {
            context,
            failures: vec![failure.into()],
        }
    }

    fn many(context: &'static str, failures: Vec<String>) -> Self {
        Self { context, failures }
    }

    fn state(error: StateError) -> Self {
        Self::single(
            "transitioning process termination coordinator",
            format!("{error:?}"),
        )
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.failures.join("; "))
    }
}

impl Error for HostError {}

const _: [i32; 4] = [SIGHUP, SIGINT, SIGQUIT, SIGTERM];

#[cfg(test)]
mod tests;
