//! Deferred Unix termination delivery and post-restoration disposition.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the signal receiver lands before its live TUI runner consumer"
    )
)]

use std::io;

use signal_hook::{
    consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM},
    iterator::Signals,
};

const TERMINATION_SIGNALS: [i32; 4] = [SIGHUP, SIGINT, SIGQUIT, SIGTERM];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminationSignal {
    Hangup,
    Interrupt,
    Quit,
    Terminate,
}

impl TerminationSignal {
    const fn from_raw(signal: i32) -> Option<Self> {
        match signal {
            SIGHUP => Some(Self::Hangup),
            SIGINT => Some(Self::Interrupt),
            SIGQUIT => Some(Self::Quit),
            SIGTERM => Some(Self::Terminate),
            _ => None,
        }
    }

    const fn as_raw(self) -> i32 {
        match self {
            Self::Hangup => SIGHUP,
            Self::Interrupt => SIGINT,
            Self::Quit => SIGQUIT,
            Self::Terminate => SIGTERM,
        }
    }
}

pub(super) trait SignalSource {
    fn pending(&mut self) -> Option<TerminationSignal>;
}

pub(super) struct TerminationSignals {
    signals: Signals,
}

impl TerminationSignals {
    pub(super) fn register() -> io::Result<Self> {
        Signals::new(TERMINATION_SIGNALS).map(|signals| Self { signals })
    }
}

impl SignalSource for TerminationSignals {
    fn pending(&mut self) -> Option<TerminationSignal> {
        self.signals.pending().find_map(TerminationSignal::from_raw)
    }
}

#[cfg(test)]
mod tests;
