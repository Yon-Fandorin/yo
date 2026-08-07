//! Independent process-termination, terminal-input, and owner-thread wait adapters.

use std::{
    task::{Context, Poll},
    thread,
    time::Duration,
};

use super::input::{EventSource, InputReadFailure, InputReader};
use crate::{
    input::event::InputEvent,
    runner::{TerminationEvent, TerminationSource},
};

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

    pub(crate) fn poll_termination(&mut self, context: &mut Context<'_>) -> Poll<TerminationEvent> {
        self.termination.poll_termination(context)
    }

    pub(crate) fn poll_input(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<InputEvent, InputReadFailure<E::Error>>> {
        self.input.poll_event(context)
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>) {
        match timeout {
            Some(timeout) => thread::park_timeout(timeout),
            None => thread::park(),
        }
    }
}

#[cfg(test)]
mod tests;
