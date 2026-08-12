//! Independent process-termination, terminal-input, and owner-thread wait adapters.

use std::{
    collections::VecDeque,
    task::{Context, Poll},
    thread,
    time::Duration,
};

use super::input::{EventSource, InputReadFailure, InputReader};
use crate::{
    input::event::InputEvent,
    runner::{TerminationEvent, TerminationSource},
};

const POST_FLUSH_EVENT_LIMIT: usize = 4_096;

pub(crate) struct UnixEventReader<E, T> {
    input: InputReader<E>,
    termination: T,
    pending_input: VecDeque<InputEvent>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PostFlushResizes {
    pub(crate) count: u64,
    pub(crate) latest: Option<crate::surface::Size>,
}

#[derive(Debug)]
pub(crate) enum PostFlushObservationError<SourceError> {
    Input(InputReadFailure<SourceError>),
    ResizeCountOverflow,
    EventLimitExceeded { limit: usize },
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
            pending_input: VecDeque::new(),
        }
    }

    pub(crate) fn poll_termination(&mut self, context: &mut Context<'_>) -> Poll<TerminationEvent> {
        self.termination.poll_termination(context)
    }

    pub(crate) fn poll_input(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<InputEvent, InputReadFailure<E::Error>>> {
        if let Some(input) = self.pending_input.pop_front() {
            Poll::Ready(Ok(input))
        } else {
            self.input.poll_event(context)
        }
    }

    pub(crate) fn observe_post_flush_resizes(
        &mut self,
        context: &mut Context<'_>,
    ) -> Result<PostFlushResizes, PostFlushObservationError<E::Error>> {
        let mut resizes = PostFlushResizes::default();
        for _ in 0..POST_FLUSH_EVENT_LIMIT {
            match self.input.poll_event(context) {
                Poll::Pending => return Ok(resizes),
                Poll::Ready(Err(error)) => {
                    return Err(PostFlushObservationError::Input(error));
                },
                Poll::Ready(Ok(InputEvent::Resize(size))) => {
                    resizes.count = resizes
                        .count
                        .checked_add(1)
                        .ok_or(PostFlushObservationError::ResizeCountOverflow)?;
                    resizes.latest = Some(size);
                },
                Poll::Ready(Ok(input)) => self.pending_input.push_back(input),
            }
        }
        Err(PostFlushObservationError::EventLimitExceeded {
            limit: POST_FLUSH_EVENT_LIMIT,
        })
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
