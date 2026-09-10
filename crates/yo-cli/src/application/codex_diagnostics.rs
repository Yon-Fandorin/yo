use std::{
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll, Waker},
};

use yo_backend_delegated_codex::{CodexCompatibilityWarning, CodexWarningObserver};
use yo_core::{ActivityNotice, NoticeLevel};
use yo_tui::{AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch};

use super::output::write_cli_diagnostics;
use crate::interaction::diagnostic::{AppError, CliDiagnostic};

pub(super) const MAX_CODEX_COMPATIBILITY_WARNINGS: usize = 32;

#[derive(Clone, Default)]
pub(super) struct CodexWarningCollector {
    state: Arc<Mutex<CodexWarningCollectorState>>,
}

#[derive(Default)]
struct CodexWarningCollectorState {
    warnings: Vec<CollectedWarning>,
    seen_messages: Vec<String>,
    published: usize,
    suppressed: bool,
    suppression_published: bool,
    disabled: bool,
    waker: Option<Waker>,
}

struct CollectedWarning {
    display: String,
    notice: ActivityNotice,
}

impl CodexWarningCollector {
    pub(super) fn observer(&self) -> CodexWarningObserver {
        let collector = self.clone();
        Arc::new(move |warning| collector.observe(warning))
    }

    fn observe(&self, warning: CodexCompatibilityWarning) {
        self.observe_warning(warning.to_string(), warning.to_notice());
    }

    #[cfg(test)]
    fn observe_message(&self, message: String) {
        let notice = ActivityNotice {
            title: "Codex warning".to_owned(),
            message: message.clone(),
            level: NoticeLevel::Warning,
        };
        self.observe_warning(message, notice);
    }

    fn observe_warning(&self, message: String, notice: ActivityNotice) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.disabled || state.seen_messages.iter().any(|seen| seen == &message) {
            return;
        }
        if state.warnings.len() < MAX_CODEX_COMPATIBILITY_WARNINGS {
            state.seen_messages.push(message.clone());
            state.warnings.push(CollectedWarning {
                display: message,
                notice,
            });
        } else if !state.suppressed {
            state.suppressed = true;
        } else {
            return;
        }
        let waker = state.waker.take();
        drop(state);
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(super) fn connection<'a, A: AgentConnection>(
        &'a self,
        agent: &'a mut A,
        pending_poll: &'a mut Option<Result<AgentPoll, A::Error>>,
    ) -> impl AgentConnection<Error = A::Error> + 'a {
        DiagnosticConnection {
            collector: self,
            agent,
            pending_poll,
        }
    }

    fn take_notice(&self) -> Option<ActivityNotice> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(notice) = state
            .warnings
            .get(state.published)
            .map(|warning| warning.notice.clone())
        {
            state.published += 1;
            return Some(notice);
        }
        if state.suppressed && !state.suppression_published {
            state.suppression_published = true;
            return Some(ActivityNotice {
                title: "Codex warning".to_owned(),
                message: format!(
                    "additional Codex warnings were suppressed after {MAX_CODEX_COMPATIBILITY_WARNINGS} distinct warnings"
                ),
                level: NoticeLevel::Warning,
            });
        }
        None
    }

    fn poll_notice_ready(&self, context: &mut Context<'_>) -> Poll<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.published < state.warnings.len()
            || (state.suppressed && !state.suppression_published)
        {
            return Poll::Ready(());
        }
        state.waker = Some(context.waker().clone());
        Poll::Pending
    }

    fn take_pending_diagnostics(&self) -> Vec<CliDiagnostic> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = state.warnings[state.published..]
            .iter()
            .map(|warning| CliDiagnostic::warning(warning.display.clone()))
            .collect::<Vec<_>>();
        state.published = state.warnings.len();
        let mut diagnostics = pending;
        if state.suppressed && !state.suppression_published {
            state.suppression_published = true;
            diagnostics.push(CliDiagnostic::warning(format!(
                "additional Codex warnings were suppressed after {MAX_CODEX_COMPATIBILITY_WARNINGS} distinct warnings"
            )));
        }
        diagnostics
    }

    pub(super) fn discard_pending(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.published = state.warnings.len();
        state.suppressed = false;
        state.suppression_published = true;
        state.disabled = true;
    }
}

struct DiagnosticConnection<'a, A: AgentConnection> {
    collector: &'a CodexWarningCollector,
    agent: &'a mut A,
    pending_poll: &'a mut Option<Result<AgentPoll, A::Error>>,
}

impl<A: AgentConnection> AgentConnection for DiagnosticConnection<'_, A> {
    type Error = A::Error;
    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        self.agent.dispatch(action)
    }
    fn retry(&mut self, pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        self.agent.retry(pending)
    }
    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        if let Some(notice) = self.collector.take_notice() {
            return Ok(AgentPoll::Notice(notice));
        }
        if let Some(pending) = self.pending_poll.take() {
            return pending;
        }
        let result = self.agent.poll();
        // Receiving can publish warnings before yielding a record, closure, or failure.
        // The Session retains that result across temporary frontend connections.
        if let Some(notice) = self.collector.take_notice() {
            *self.pending_poll = Some(result);
            Ok(AgentPoll::Notice(notice))
        } else {
            result
        }
    }
    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        if self.pending_poll.is_some() || self.collector.poll_notice_ready(context).is_ready() {
            Poll::Ready(())
        } else {
            self.agent.poll_ready(context)
        }
    }
}

impl<A: AgentConnection> Drop for DiagnosticConnection<'_, A> {
    fn drop(&mut self) {
        self.collector
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .waker = None;
    }
}

pub(super) fn publish_pending_codex_diagnostics(
    collector: &CodexWarningCollector,
) -> Result<(), AppError> {
    let diagnostics = collector.take_pending_diagnostics();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        write_cli_diagnostics(&diagnostics)
    }
}

pub(super) fn error_after_codex_diagnostics(
    error: AppError,
    collector: &CodexWarningCollector,
) -> AppError {
    match publish_pending_codex_diagnostics(collector) {
        Ok(()) => error,
        Err(diagnostics_error) => AppError::combine([error, diagnostics_error]),
    }
}

#[cfg(test)]
mod tests;
