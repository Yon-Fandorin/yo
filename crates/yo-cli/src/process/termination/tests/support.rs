use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use nix::sys::signal::Signal;

use super::super::{SignalOs, TerminationCoordinator, state::SharedState};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Call {
    CaptureAndBlock,
    Install(Signal),
    Unblock,
    Block,
    Restore(Signal),
    RestoreMask,
}

#[derive(Clone)]
pub(super) struct RecordingOs {
    calls: Rc<RefCell<Vec<Call>>>,
    failures: BTreeSet<Call>,
}

impl RecordingOs {
    pub(super) fn new(failures: impl IntoIterator<Item = Call>) -> (Self, Rc<RefCell<Vec<Call>>>) {
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

pub(super) fn shared() -> &'static SharedState {
    Box::leak(Box::new(SharedState::installing()))
}

pub(super) fn coordinator(
    failures: impl IntoIterator<Item = Call>,
) -> (TerminationCoordinator<RecordingOs>, Rc<RefCell<Vec<Call>>>) {
    let shared = shared();
    let (os, calls) = RecordingOs::new(failures);
    let coordinator = TerminationCoordinator::install_with(shared, os, false).unwrap();
    (coordinator, calls)
}
