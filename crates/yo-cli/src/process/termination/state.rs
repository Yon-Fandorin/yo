use std::sync::atomic::{AtomicUsize, Ordering};

use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

const PHASE_MASK: usize = 0xff;
const HUP_BIT: usize = 1 << 8;
const INT_BIT: usize = 1 << 9;
const QUIT_BIT: usize = 1 << 10;
const TERM_BIT: usize = 1 << 11;
const PENDING_MASK: usize = HUP_BIT | INT_BIT | QUIT_BIT | TERM_BIT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(super) enum Phase {
    New = 0,
    Installing = 1,
    Idle = 2,
    Active = 3,
    Cleaning = 4,
    Terminating = 5,
    ShuttingDown = 6,
    Retired = 7,
    FailedRetired = 8,
}

impl Phase {
    fn from_word(word: usize) -> Self {
        match word & PHASE_MASK {
            0 => Self::New,
            1 => Self::Installing,
            2 => Self::Idle,
            3 => Self::Active,
            4 => Self::Cleaning,
            5 => Self::Terminating,
            6 => Self::ShuttingDown,
            7 => Self::Retired,
            8 => Self::FailedRetired,
            _ => Self::FailedRetired,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Publication {
    Published,
    DefaultNow,
}

pub(super) struct SharedState {
    word: AtomicUsize,
}

impl SharedState {
    pub(super) const fn new() -> Self {
        Self {
            word: AtomicUsize::new(Phase::New as usize),
        }
    }

    #[cfg(test)]
    pub(super) fn installing() -> Self {
        Self {
            word: AtomicUsize::new(Phase::Installing as usize),
        }
    }

    pub(super) fn observe_termination(&self) -> bool {
        self.word.load(Ordering::SeqCst) & PENDING_MASK != 0
    }

    pub(super) fn publish(&self, signal: i32) -> Publication {
        let bit = signal_bit(signal);
        let mut current = self.word.load(Ordering::SeqCst);
        loop {
            match Phase::from_word(current) {
                Phase::Active | Phase::Cleaning => {
                    let next = current | bit;
                    match self.word.compare_exchange_weak(
                        current,
                        next,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => return Publication::Published,
                        Err(observed) => current = observed,
                    }
                },
                _ => return Publication::DefaultNow,
            }
        }
    }

    pub(super) fn transition_preserving(&self, from: Phase, to: Phase) -> Result<(), StateError> {
        let mut current = self.word.load(Ordering::SeqCst);
        loop {
            let phase = Phase::from_word(current);
            if phase != from {
                return Err(StateError::UnexpectedPhase {
                    expected: from,
                    actual: phase,
                });
            }
            let next = (current & !PHASE_MASK) | to as usize;
            match self
                .word
                .compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    pub(super) fn finalize_cleaning(&self) -> Result<Option<i32>, StateError> {
        let mut current = self.word.load(Ordering::SeqCst);
        loop {
            let phase = Phase::from_word(current);
            if phase != Phase::Cleaning {
                return Err(StateError::UnexpectedPhase {
                    expected: Phase::Cleaning,
                    actual: phase,
                });
            }
            let selected = select_signal(current);
            let next = match selected {
                Some(_) => Phase::Terminating,
                None => Phase::Idle,
            } as usize;
            match self
                .word
                .compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Ok(selected),
                Err(observed) => current = observed,
            }
        }
    }

    pub(super) fn force_phase(&self, phase: Phase) {
        self.word.store(phase as usize, Ordering::SeqCst);
    }

    pub(super) fn fail_retire(&self) -> Option<i32> {
        let mut current = self.word.load(Ordering::SeqCst);
        loop {
            let next = (current & !PHASE_MASK) | Phase::FailedRetired as usize;
            match self
                .word
                .compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return select_signal(next),
                Err(observed) => current = observed,
            }
        }
    }

    pub(super) fn snapshot_phase(&self) -> Phase {
        Phase::from_word(self.word.load(Ordering::SeqCst))
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> (Phase, usize) {
        let word = self.word.load(Ordering::SeqCst);
        (Phase::from_word(word), word & PENDING_MASK)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateError {
    UnexpectedPhase { expected: Phase, actual: Phase },
}

fn signal_bit(signal: i32) -> usize {
    match signal {
        SIGHUP => HUP_BIT,
        SIGINT => INT_BIT,
        SIGQUIT => QUIT_BIT,
        SIGTERM => TERM_BIT,
        _ => TERM_BIT,
    }
}

fn select_signal(word: usize) -> Option<i32> {
    [
        (HUP_BIT, SIGHUP),
        (INT_BIT, SIGINT),
        (QUIT_BIT, SIGQUIT),
        (TERM_BIT, SIGTERM),
    ]
    .into_iter()
    .find_map(|(bit, signal)| (word & bit != 0).then_some(signal))
}
