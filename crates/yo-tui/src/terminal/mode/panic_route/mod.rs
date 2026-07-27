//! Process-global panic routing for one terminal-owning boundary.
//!
//! Every yo terminal boundary must use this module. Code that replaces the
//! process hook independently while a boundary is active is outside the
//! supported integration contract because Rust exposes no atomic hook swap.

use std::{
    any::Any,
    fmt,
    io::{self, Write},
    panic::{self, PanicHookInfo, UnwindSafe},
    sync::{Arc, Mutex, TryLockError},
    thread,
};

type PanicHook = dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static;
type PanicPayload = Box<dyn Any + Send + 'static>;

static PANIC_HOOK_OWNER: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PanicDiagnostic {
    pub(crate) thread: String,
    pub(crate) message: String,
    pub(crate) location: Option<PanicLocation>,
}

impl PanicDiagnostic {
    fn capture(info: &PanicHookInfo<'_>) -> Self {
        let current = thread::current();
        let thread = current
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{:?}", current.id()));
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        let location = info.location().map(|location| PanicLocation {
            file: location.file().to_owned(),
            line: location.line(),
            column: location.column(),
        });

        Self {
            thread,
            message,
            location,
        }
    }

    pub(crate) fn emit(&self, writer: &mut impl Write) -> io::Result<()> {
        writeln!(writer, "{self}")
    }
}

impl fmt::Display for PanicDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "thread '{}' panicked", self.thread)?;
        if let Some(location) = &self.location {
            write!(
                formatter,
                " at {}:{}:{}",
                location.file, location.line, location.column
            )?;
        }
        write!(formatter, ":\n{}", self.message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PanicLocation {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanicRouteError {
    AlreadyInstalled,
    OwnershipPoisoned,
}

pub(crate) struct PanicOutcome<T> {
    pub(crate) result: Result<T, PanicPayload>,
    pub(crate) diagnostic: Option<PanicDiagnostic>,
}

/// Runs the complete terminal boundary under one owner-thread panic route.
///
/// `operation` must contain the inner application catch, explicit terminal
/// cleanup, and `resume_unwind` sequence. `resume_unwind` does not invoke the
/// hook again, so the first diagnostic remains captured until restoration.
pub(crate) fn catch_owner_panic<T>(
    operation: impl FnOnce() -> T + UnwindSafe,
) -> Result<PanicOutcome<T>, PanicRouteError> {
    let ownership = match PANIC_HOOK_OWNER.try_lock() {
        Ok(ownership) => ownership,
        Err(TryLockError::WouldBlock) => return Err(PanicRouteError::AlreadyInstalled),
        Err(TryLockError::Poisoned(_)) => return Err(PanicRouteError::OwnershipPoisoned),
    };
    let owner = thread::current().id();
    let captured = Arc::new(Mutex::new(None));
    let previous: Arc<PanicHook> = panic::take_hook().into();

    let hook_captured = Arc::clone(&captured);
    let hook_previous = Arc::clone(&previous);
    panic::set_hook(Box::new(move |info| {
        if thread::current().id() == owner {
            let mut slot = hook_captured
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(PanicDiagnostic::capture(info));
            }
        } else {
            hook_previous(info);
        }
    }));

    let result = panic::catch_unwind(operation);

    // `catch_unwind` has ended owner-thread unwinding, so restoring the
    // process-global hook here cannot trigger `set_hook`'s double-panic path.
    panic::set_hook(Box::new(move |info| previous(info)));
    let diagnostic = captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    drop(ownership);

    Ok(PanicOutcome { result, diagnostic })
}

#[cfg(test)]
mod tests;
