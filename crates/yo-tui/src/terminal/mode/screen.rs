use std::iter;

use super::transaction::{SessionFailure, TerminalSession};
use crate::terminal::backend::{ScreenModeBackend, TerminalBackend};

type EntryFailure<B> = SessionFailure<
    <B as TerminalBackend>::Error,
    <B as TerminalBackend>::Mode,
    <B as TerminalBackend>::Error,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScreenMode {
    Inline,
    Fullscreen,
}

pub(crate) fn enter_screen<B>(
    backend: &mut B,
    screen_mode: ScreenMode,
) -> Result<TerminalSession<'_, B>, EntryFailure<B>>
where
    B: ScreenModeBackend,
{
    match screen_mode {
        ScreenMode::Inline => TerminalSession::enter(backend, iter::empty()),
        ScreenMode::Fullscreen => {
            TerminalSession::enter(backend, iter::once(B::alternate_screen_mode()))
        },
    }
}
