use crate::diagnostic::CliDiagnostic;

pub(crate) struct Output {
    pub(crate) stdout: String,
    pub(crate) diagnostics: Vec<CliDiagnostic>,
}
