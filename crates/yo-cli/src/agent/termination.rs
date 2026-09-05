use std::task::{Context, Poll};

use yo_tui::{TerminationEvent, TerminationSource};

pub(in crate::agent) fn requested(termination: &mut impl TerminationSource) -> bool {
    let waker = std::task::Waker::noop();
    let mut context = Context::from_waker(waker);
    termination.poll_termination(&mut context) == Poll::Ready(TerminationEvent::Requested)
}
