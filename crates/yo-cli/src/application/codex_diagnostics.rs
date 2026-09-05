use std::sync::{Arc, Mutex};

use super::output::write_cli_diagnostics;
use crate::interaction::diagnostic::{AppError, CliDiagnostic};

pub(super) const MAX_CODEX_COMPATIBILITY_WARNINGS: usize = 32;

#[derive(Clone, Default)]
pub(super) struct CodexWarningCollector {
    state: Arc<Mutex<CodexWarningCollectorState>>,
}

#[derive(Default)]
struct CodexWarningCollectorState {
    warnings: Vec<String>,
    seen_messages: Vec<String>,
    published: usize,
    suppressed: bool,
    suppression_published: bool,
    disabled: bool,
}

impl CodexWarningCollector {
    pub(super) fn observer(&self) -> yo_backend_delegated_codex::CodexWarningObserver {
        let collector = self.clone();
        Arc::new(move |warning| collector.observe(warning))
    }

    fn observe(&self, warning: yo_backend_delegated_codex::CodexCompatibilityWarning) {
        self.observe_message(warning.to_string());
    }

    fn observe_message(&self, message: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.disabled || state.seen_messages.iter().any(|seen| seen == &message) {
            return;
        }
        if state.warnings.len() < MAX_CODEX_COMPATIBILITY_WARNINGS {
            state.seen_messages.push(message.clone());
            state.warnings.push(message);
        } else {
            state.suppressed = true;
        }
    }

    fn take_pending_diagnostics(&self) -> Vec<CliDiagnostic> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pending = state.warnings[state.published..]
            .iter()
            .map(|warning| CliDiagnostic::warning(warning.clone()))
            .collect::<Vec<_>>();
        state.published = state.warnings.len();
        let mut diagnostics = pending;
        if state.suppressed && !state.suppression_published {
            state.suppression_published = true;
            diagnostics.push(CliDiagnostic::warning(format!(
                "additional Codex compatibility warnings were suppressed after {MAX_CODEX_COMPATIBILITY_WARNINGS} distinct warnings"
            )));
        }
        diagnostics
    }

    pub(super) fn discard_pending(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.published = state.warnings.len();
        state.suppressed = false;
        state.suppression_published = true;
        state.disabled = true;
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
