//! Fair polling of process-host termination and terminal input on the owner thread.

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

    pub(crate) fn poll_next(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<UnixEvent, InputReadFailure<E::Error>>> {
        if self.termination.poll_termination() == TerminationEvent::Requested {
            return Poll::Ready(Ok(UnixEvent::Terminate));
        }

        let input = self.input.poll_event(context);
        if self.termination.poll_termination() == TerminationEvent::Requested {
            return Poll::Ready(Ok(UnixEvent::Terminate));
        }

        input.map(|result| result.map(UnixEvent::Input))
    }

    pub(crate) fn next(
        &mut self,
        timeout: Duration,
        context: &mut Context<'_>,
    ) -> Result<UnixEvent, InputReadFailure<E::Error>> {
        if let Poll::Ready(event) = self.poll_next(context) {
            return event;
        }

        thread::park_timeout(timeout);
        match self.poll_next(context) {
            Poll::Ready(event) => event,
            Poll::Pending => Ok(UnixEvent::Idle),
        }
    }
}

#[cfg(test)]
mod tests;
