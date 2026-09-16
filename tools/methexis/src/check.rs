use std::path::Path;

#[cfg(test)]
use crate::checkpoint;

const CHECK_SCHEMA: &str = "methexis.check/v1alpha1";
const MAX_RECORD_BYTES: usize = 256 * 1024;

pub(crate) mod artifacts;
mod body;
mod cycles;
mod foundation;
mod model;
mod revision;
mod runner;
mod validation;

#[cfg(test)]
mod tests;

pub(crate) use body::{body_has_forbidden_html, body_start_line};
use foundation::load_records;
pub(crate) use foundation::{collect_files, normalize_record_bytes, parse_yaml, read_normalized};
#[cfg(test)]
use foundation::{normalize_line_endings, split_frontmatter};
pub(crate) use model::Foundation;
pub use model::{
    CheckClass, CheckOutcome, CheckReport, CheckStatus, Diagnostic, DiagnosticPhase, UnitRevision,
};
pub(crate) use revision::knowledge_revision;
use revision::snapshot_revision;
use validation::{
    display_path, global_diagnostic, local_diagnostic, sort_diagnostics, validate_global,
};
pub(crate) use validation::{is_segment, is_semantic_id, valid_hash, validate_metadata};

pub(crate) fn check_repository(repository_root: &Path) -> CheckReport {
    runner::check_repository_selected(repository_root, &CheckClass::ALL)
}

pub(crate) fn check_repository_selected(
    repository_root: &Path,
    requested: &[CheckClass],
) -> CheckReport {
    runner::check_repository_selected(repository_root, requested)
}

pub(crate) fn load_foundation(repository_root: &Path) -> Result<Foundation, Vec<Diagnostic>> {
    let foundation = load_records(repository_root)?;
    let mut global_diagnostics = validate_global(
        &foundation.units,
        &foundation.owners,
        &foundation.sources,
        &foundation.negative_records,
        repository_root,
    );
    sort_diagnostics(&mut global_diagnostics);
    if !global_diagnostics.is_empty() {
        return Err(global_diagnostics);
    }
    Ok(foundation)
}

#[cfg(test)]
pub(crate) fn failed_authority_report(failure: checkpoint::AuthorityFailure) -> CheckReport {
    runner::failed_authority_report(failure)
}
