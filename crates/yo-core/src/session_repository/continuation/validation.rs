//! 연속성 식별자, 앵커, 제한을 검사하는 순수 모듈.

use crate::session_repository::RepositoryError;

pub(super) fn validate_limits(
    physical_bytes: u64,
    physical_records: usize,
    returned_boundaries: usize,
) -> Result<(), RepositoryError> {
    if !(1..=256 * 1024 * 1024).contains(&physical_bytes)
        || !(1..=65_536).contains(&physical_records)
        || !(1..=256).contains(&returned_boundaries)
    {
        return Err(RepositoryError::Unavailable {
            message: "historical fork limits require 1–268435456 bytes, 1–65536 physical records, and 1–256 returned boundaries".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn selection_identity_matches(
    capture_durability_matches: bool,
    capture_session_matches: bool,
) -> bool {
    capture_durability_matches && capture_session_matches
}

pub(super) fn boundary_epochs_match(
    binding_epoch: Option<u64>,
    context_epoch: Option<u64>,
    expected_binding_epoch: u64,
    expected_context_epoch: u64,
) -> bool {
    binding_epoch == Some(expected_binding_epoch) && context_epoch == Some(expected_context_epoch)
}

pub(super) fn source_epoch_matches_open_epoch(
    source_epoch: u64,
    open_epoch: u64,
    replacement_source: bool,
) -> bool {
    source_epoch == open_epoch || replacement_source
}
