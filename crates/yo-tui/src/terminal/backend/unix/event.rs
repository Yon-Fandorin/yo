//! Fair polling of process signals and terminal input on the owner thread.

use std::time::Duration;

use super::{
    input::{EventSource, InputReadFailure, InputReader},
    signal::{SignalSource, TerminationSignal},
};
use crate::input::event::InputEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum UnixEvent {
    Input(InputEvent),
    Terminate(TerminationSignal),
    Idle,
}

pub(super) struct UnixEventReader<E, S> {
    input: InputReader<E>,
    signals: S,
}

impl<E, S> UnixEventReader<E, S>
where
    E: EventSource,
    S: SignalSource,
{
    pub(super) fn new(input: InputReader<E>, signals: S) -> Self {
        Self { input, signals }
    }

    pub(super) fn next(
        &mut self,
        timeout: Duration,
    ) -> Result<UnixEvent, InputReadFailure<E::Error>> {
        if let Some(signal) = self.signals.pending() {
            return Ok(UnixEvent::Terminate(signal));
        }

        let input = self.input.poll(timeout);
        if let Some(signal) = self.signals.pending() {
            return Ok(UnixEvent::Terminate(signal));
        }

        Ok(input?.map_or(UnixEvent::Idle, UnixEvent::Input))
    }
}

#[cfg(test)]
mod tests;
