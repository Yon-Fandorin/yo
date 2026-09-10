//! Deterministic completed cell state for yo terminal interfaces.

pub(crate) mod appearance;
pub(crate) mod command;
pub mod html;
pub(crate) mod input;
pub(crate) mod layout;
pub mod meter;
pub(crate) mod overlay;
pub mod plain;
pub(crate) mod prompt;
#[cfg(unix)]
mod runner;
pub(crate) mod shell;
pub mod surface;
pub mod terminal;
pub(crate) mod text;
pub(crate) mod transcript;

pub use appearance::{
    ColorCapability, GlyphProfile, MotionPreference, OutputPreferences, Theme, ThemeColor,
    ThemeOverrides, ThemeRole,
};
pub use command::{PromptTemplateError, PromptTemplates};
#[cfg(unix)]
pub use runner::{
    AgentAction, AgentConnection, AgentPoll, ArchivedContentPolicy, ArchivedProjectionError,
    ArchivedProjectionOptions, ArchivedSessionView, DispatchOutcome, ExitReason, ForkPickerToken,
    FrameRateLimit, PendingDispatch, PresentationMode, PublicationRecoveryEvidence,
    PublicationRecoveryKind, ResumeSessionEntry, RunError, RunOutcome, TerminalOutcome,
    TerminationEvent, TerminationSource, TuiDocument, TuiSession, TuiSessionInfo, TuiStatusError,
    TuiStatusLine, WorkspaceReferenceConnection, WorkspaceReferencePoll, project_archived_session,
    project_archived_session_with_options, project_archived_usage, run, run_session_with_mode,
    run_with_mode,
};
pub use transcript::{
    AssistantRenderInput, AssistantRenderer, DocumentRenderInput, DocumentRenderer, LinkResolver,
    ToolRenderInput, ToolRenderer, TranscriptActivityOutcome,
};
