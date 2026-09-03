use std::{fmt, sync::Arc};

use crate::{BackendBindingEvidence, BackendCommandEvidence};

/// Frontend-independent port implemented by an agent provider adapter.
///
/// Provider wire types remain behind this boundary. An implementation may use a worker internally,
/// but these methods themselves do not expose transport or process details.
pub trait BackendAdapter {
    type Command;
    type Event;
    type ResumeTarget;

    /// Returns a thread-safe handle that can break an outstanding backend operation.
    ///
    /// Calling the handle must be prompt and idempotent. After it is called, any currently
    /// executing backend method must return promptly so that its owner can run [`Self::shutdown`].
    fn stop_handle(&self) -> BackendStopHandle;

    /// Returns provider-neutral capabilities fixed for this initialized backend.
    fn capabilities(&self) -> BackendCapabilities;

    /// Reconnects an existing durable binding before ordinary command admission begins.
    ///
    /// Adapters that do not support native resume fail closed through the default implementation.
    fn resume_session(
        &mut self,
        _target: &Self::ResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "this backend does not support native Session resume",
        ))
    }

    /// Opens a new binding from the exact replay carried by a durable anchor.
    fn resume_session_replacing_binding(
        &mut self,
        _target: &Self::ResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "this backend does not support exact-replay binding replacement",
        ))
    }

    /// Forks provider-managed Session state into a distinct binding using a configured model.
    ///
    /// Adapters must advertise [`BackendCapabilities::supports_native_model_rebind`] before this
    /// method can be called. The configured target model remains adapter-owned wire state; the
    /// durable source coordinate is carried by `target`.
    fn resume_session_rebinding_model(
        &mut self,
        _target: &Self::ResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "this backend does not support native model rebinding",
        ))
    }

    /// Executes a command far enough to know whether the backend accepted it.
    ///
    /// The returned provider-neutral evidence contains only facts the adapter observed; the
    /// runtime owns epochs, operation identities, and durable coordinates. `None` means the
    /// backend accepted the command without claiming resumable correlation. Streamed work caused
    /// by an accepted command is observed through [`Self::poll_event`].
    fn execute_command(
        &mut self,
        command: Self::Command,
    ) -> Result<BackendCommandEvidence, BackendFailure>;

    /// Observes one already available semantic event without waiting for future backend work.
    fn poll_event(&mut self) -> Result<BackendPoll<Self::Event>, BackendFailure>;

    /// Explicitly releases backend-owned resources.
    ///
    /// Implementations must make repeated calls safe. A successful call makes later polling return
    /// [`BackendPoll::Closed`] and later commands fail explicitly.
    fn shutdown(&mut self) -> Result<(), BackendFailure>;
}

impl<T> BackendAdapter for Box<T>
where
    T: BackendAdapter + ?Sized,
{
    type Command = T::Command;
    type Event = T::Event;
    type ResumeTarget = T::ResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        (**self).stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        (**self).capabilities()
    }

    fn resume_session(
        &mut self,
        target: &Self::ResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        (**self).resume_session(target)
    }

    fn resume_session_replacing_binding(
        &mut self,
        target: &Self::ResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        (**self).resume_session_replacing_binding(target)
    }

    fn resume_session_rebinding_model(
        &mut self,
        target: &Self::ResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        (**self).resume_session_rebinding_model(target)
    }

    fn execute_command(
        &mut self,
        command: Self::Command,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        (**self).execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll<Self::Event>, BackendFailure> {
        (**self).poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        (**self).shutdown()
    }
}

/// Provider-neutral, cloneable cancellation handle for backend lifecycle ownership.
#[derive(Clone)]
pub struct BackendStopHandle {
    request: Arc<dyn Fn() + Send + Sync>,
}

impl BackendStopHandle {
    /// Creates a stop handle from an idempotent, nonblocking callback.
    pub fn new(request: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            request: Arc::new(request),
        }
    }

    /// Creates a handle for a backend whose operations always return promptly.
    #[must_use]
    pub fn no_op() -> Self {
        Self::new(|| {})
    }

    /// Requests cancellation without waiting for backend cleanup.
    pub fn request_stop(&self) {
        (self.request)();
    }
}

impl fmt::Debug for BackendStopHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackendStopHandle(..)")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BackendCapabilities {
    steer: bool,
    native_model_rebind: bool,
}

impl BackendCapabilities {
    pub const fn none() -> Self {
        Self {
            steer: false,
            native_model_rebind: false,
        }
    }

    pub const fn with_steer(mut self) -> Self {
        self.steer = true;
        self
    }

    pub const fn supports_steer(self) -> bool {
        self.steer
    }

    pub const fn with_native_model_rebind(mut self) -> Self {
        self.native_model_rebind = true;
        self
    }

    pub const fn supports_native_model_rebind(self) -> bool {
        self.native_model_rebind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendPoll<Event> {
    Pending,
    Event(Event),
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendFailureKind {
    Unavailable,
    Initialization,
    Session,
    CommandRejected,
    Unsupported,
    Protocol,
    ProcessExit,
    Turn,
    ContextExhausted,
    Cleanup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendFailure {
    kind: BackendFailureKind,
    message: String,
}

impl BackendFailure {
    pub fn new(kind: BackendFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> BackendFailureKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BackendFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for BackendFailure {}
