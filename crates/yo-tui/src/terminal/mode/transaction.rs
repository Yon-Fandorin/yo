use std::{
    any::Any,
    mem,
    panic::{AssertUnwindSafe, catch_unwind},
};

use crate::terminal::backend::{TerminalBackend, TerminalOutputBackend};

type BackendFailure<B> = SessionFailure<
    SessionFailureCause<<B as TerminalBackend>::Error>,
    <B as TerminalBackend>::Mode,
    <B as TerminalBackend>::Error,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CleanupStep<M> {
    Mode(M),
    TtyState,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CleanupFailure<M, E> {
    pub(crate) step: CleanupStep<M>,
    pub(crate) cause: CleanupFailureCause<E>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CleanupFailureCause<E> {
    Error(E),
    Panicked(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CleanupFailures<M, E> {
    pub(crate) failures: Vec<CleanupFailure<M, E>>,
}

pub(crate) enum SessionFailureCause<E> {
    Error(E),
    Panicked(Box<dyn Any + Send>),
}

impl<E> std::fmt::Debug for SessionFailureCause<E>
where
    E: std::fmt::Debug,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error(error) => formatter.debug_tuple("Error").field(error).finish(),
            Self::Panicked(_) => formatter.write_str("Panicked(..)"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SessionFailure<P, M, E> {
    pub(crate) primary: P,
    pub(crate) cleanup: Vec<CleanupFailure<M, E>>,
}

pub(crate) struct TerminalSession<'backend, B>
where
    B: TerminalBackend,
{
    backend: &'backend mut B,
    tty_state: Option<B::TtyState>,
    acquired_modes: Vec<B::Mode>,
}

impl<'backend, B> TerminalSession<'backend, B>
where
    B: TerminalBackend,
{
    pub(crate) fn enter(
        backend: &'backend mut B,
        modes: impl IntoIterator<Item = B::Mode>,
    ) -> Result<Self, BackendFailure<B>> {
        let tty_state = match catch_unwind(AssertUnwindSafe(|| backend.capture_tty_state())) {
            Ok(Ok(state)) => state,
            Ok(Err(error)) => {
                return Err(SessionFailure {
                    primary: SessionFailureCause::Error(error),
                    cleanup: Vec::new(),
                });
            },
            Err(payload) => {
                return Err(SessionFailure {
                    primary: SessionFailureCause::Panicked(payload),
                    cleanup: Vec::new(),
                });
            },
        };
        let mut session = Self {
            backend,
            tty_state: Some(tty_state),
            acquired_modes: Vec::new(),
        };

        match catch_unwind(AssertUnwindSafe(|| {
            session
                .backend
                .enable_raw_input(session.tty_state.as_ref().unwrap())
        })) {
            Ok(Ok(())) => {},
            Ok(Err(error)) => {
                return Err(session.finish_with_error(SessionFailureCause::Error(error)));
            },
            Err(payload) => {
                return Err(session.finish_with_error(SessionFailureCause::Panicked(payload)));
            },
        }

        for mode in modes {
            session.acquired_modes.push(mode);
            match catch_unwind(AssertUnwindSafe(|| session.backend.acquire_mode(mode))) {
                Ok(Ok(())) => {},
                Ok(Err(error)) => {
                    return Err(session.finish_with_error(SessionFailureCause::Error(error)));
                },
                Err(payload) => {
                    return Err(session.finish_with_error(SessionFailureCause::Panicked(payload)));
                },
            }
        }

        Ok(session)
    }

    pub(crate) fn close(mut self) -> Result<(), CleanupFailures<B::Mode, B::Error>> {
        let failures = self.restore();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CleanupFailures { failures })
        }
    }

    pub(crate) fn finish_with_error<P>(
        mut self,
        primary: P,
    ) -> SessionFailure<P, B::Mode, B::Error> {
        SessionFailure {
            primary,
            cleanup: self.restore(),
        }
    }

    pub(crate) fn owns_mode(&self, mode: B::Mode) -> bool
    where
        B::Mode: PartialEq,
    {
        self.acquired_modes.contains(&mode)
    }

    fn restore(&mut self) -> Vec<CleanupFailure<B::Mode, B::Error>> {
        let modes = mem::take(&mut self.acquired_modes);
        let tty_state = self.tty_state.take();
        let mut failures = Vec::new();

        for mode in modes.into_iter().rev() {
            if let Some(failure) =
                attempt_cleanup(CleanupStep::Mode(mode), || self.backend.release_mode(mode))
            {
                failures.push(failure);
            }
        }

        if let Some(state) = tty_state
            && let Some(failure) = attempt_cleanup(CleanupStep::TtyState, || {
                self.backend.restore_tty_state(&state)
            })
        {
            failures.push(failure);
        }

        failures
    }

    fn restore_best_effort(&mut self) {
        let modes = mem::take(&mut self.acquired_modes);
        let tty_state = self.tty_state.take();

        for mode in modes.into_iter().rev() {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _ = self.backend.release_mode(mode);
            }));
        }

        if let Some(state) = tty_state {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _ = self.backend.restore_tty_state(&state);
            }));
        }
    }
}

impl<B> TerminalSession<'_, B>
where
    B: TerminalOutputBackend,
{
    pub(crate) fn output(&mut self) -> &mut B::Output {
        self.backend.output()
    }
}

fn attempt_cleanup<M, E>(
    step: CleanupStep<M>,
    cleanup: impl FnOnce() -> Result<(), E>,
) -> Option<CleanupFailure<M, E>> {
    match catch_unwind(AssertUnwindSafe(cleanup)) {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(CleanupFailure {
            step,
            cause: CleanupFailureCause::Error(error),
        }),
        Err(payload) => Some(CleanupFailure {
            step,
            cause: CleanupFailureCause::Panicked(panic_message(payload)),
        }),
    }
}

pub(super) fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_owned(),
            Err(_) => "non-string cleanup panic".to_owned(),
        },
    }
}

impl<B> Drop for TerminalSession<'_, B>
where
    B: TerminalBackend,
{
    fn drop(&mut self) {
        self.restore_best_effort();
    }
}
