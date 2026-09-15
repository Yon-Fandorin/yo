use std::task::{Context, Poll, Waker};

use yo_tui::{TerminationEvent, TerminationSource};

pub(in crate::application::agent) fn requested(termination: &mut impl TerminationSource) -> bool {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    termination.poll_termination(&mut context) == Poll::Ready(TerminationEvent::Requested)
}
