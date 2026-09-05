//! Diagnostic construction, ordering, and repository-relative paths.

use std::path::Path;

use super::{Diagnostic, DiagnosticPhase};

pub(super) fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            left.phase,
            &left.path,
            &left.code,
            left.line,
            left.column,
            &left.message,
            &left.affected_ids,
        )
            .cmp(&(
                right.phase,
                &right.path,
                &right.code,
                right.line,
                right.column,
                &right.message,
                &right.affected_ids,
            ))
    });
}

pub(super) fn display_path(path: &Path, repository_root: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn local_diagnostic(
    path: String,
    code: impl Into<String>,
    message: String,
    line: Option<u64>,
    column: Option<u64>,
    affected_ids: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Local,
        path,
        code: code.into(),
        message,
        line,
        column,
        affected_ids,
    }
}

pub(super) fn global_diagnostic(
    path: String,
    code: impl Into<String>,
    message: String,
    affected_ids: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path,
        code: code.into(),
        message,
        line: None,
        column: None,
        affected_ids,
    }
}
