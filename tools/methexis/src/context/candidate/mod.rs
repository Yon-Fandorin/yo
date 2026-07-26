//! Librarian candidate capture and independent wire validation facade.

mod capture;
mod validation;

use std::path::Path;

use super::{
    hash::valid,
    wire::{CandidateReference, CandidateSet, ResolveFailure},
};

pub(super) struct CapturedCandidates {
    pub(super) hash: String,
    pub(super) set: CandidateSet,
    file: capture::CapturedFile,
}

pub(super) fn capture(
    repository_root: &Path,
    reference: &CandidateReference,
) -> Result<CapturedCandidates, ResolveFailure> {
    if !valid(&reference.hash) {
        return Err(failure(
            "invalid_candidate_hash",
            "candidate reference hash must be lowercase tagged SHA-256",
            false,
            &reference.path,
        ));
    }
    let file = capture::capture(repository_root, &reference.path)?;
    if file.hash != reference.hash {
        return Err(failure(
            "candidate_hash_mismatch",
            "candidate bytes do not match the expected hash",
            false,
            &reference.path,
        ));
    }
    let set: CandidateSet = serde_json::from_slice(&file.bytes).map_err(|error| {
        failure(
            "invalid_candidate_set",
            &error.to_string(),
            false,
            &reference.path,
        )
    })?;
    validation::validate(&set, &reference.path)?;
    Ok(CapturedCandidates {
        hash: file.hash.clone(),
        set,
        file,
    })
}

pub(super) fn final_revalidate(
    repository_root: &Path,
    captured: &CapturedCandidates,
) -> Result<(), ResolveFailure> {
    capture::final_revalidate(repository_root, &captured.file)
}

pub(super) fn semantic_id(value: &str) -> bool {
    validation::semantic_id(value)
}

fn failure(code: &str, message: &str, retryable: bool, path: &str) -> ResolveFailure {
    ResolveFailure::new(
        None,
        code,
        message,
        retryable,
        Vec::new(),
        vec![path.to_owned()],
        "correct or regenerate the Librarian candidate result and retry",
    )
}

#[cfg(test)]
fn final_revalidate_after_read(
    repository_root: &Path,
    captured: &CapturedCandidates,
    after_read: impl FnOnce(),
) -> Result<(), ResolveFailure> {
    capture::final_revalidate_after_read(repository_root, &captured.file, after_read)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
