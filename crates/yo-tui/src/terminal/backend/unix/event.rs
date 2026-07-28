//! Fair polling of process-host termination and terminal input on the owner thread.

use std::time::Duration;

use super::input::{EventSource, InputReadFailure, InputReader};
use crate::{
    input::event::InputEvent,
    runner::{TerminationEvent, TerminationSource},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UnixEvent {
    Input(InputEvent),
    Terminate,
    Idle,
}

pub(crate) struct UnixEventReader<E, T> {
    input: InputReader<E>,
    termination: T,
}

impl<E, T> UnixEventReader<E, T>
where
    E: EventSource,
    T: TerminationSource,
{
    pub(crate) fn new(input: E, termination: T) -> Self {
        Self {
            input: InputReader::new(input),
            termination,
        }
    }

    pub(crate) fn next(
        &mut self,
        timeout: Duration,
    ) -> Result<UnixEvent, InputReadFailure<E::Error>> {
        if self.termination.poll_termination() == TerminationEvent::Requested {
            return Ok(UnixEvent::Terminate);
        }

        let input = self.input.poll(timeout);
        if self.termination.poll_termination() == TerminationEvent::Requested {
            return Ok(UnixEvent::Terminate);
        }

        Ok(input?.map_or(UnixEvent::Idle, UnixEvent::Input))
    }
}

#[cfg(test)]
mod tests;
