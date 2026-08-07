//! Approval-request preparation from a published review packet manifest.
//!
//! `prepare-approval` binds the manifest's KnowledgeId, RevisionId, and
//! Projection hash into the exact ApprovalRequest wire shape so no value is
//! copied by hand between commands. It only emits the request: it never
//! writes `methexis/approvals/` and never records an approval, so human
//! authorization remains the separate explicit `approve` step.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    APPROVAL_REQUEST_SCHEMA, ApprovalRequest, OperationFailure, REVIEW_MANIFEST_SCHEMA,
    ReviewManifest, failure_from_diagnostic,
    operations::{load_operation_foundation, read_request, require_schema},
    records::parse_approval,
};

const OPERATION: &str = "prepare_approval";

pub(super) fn prepare_approval(
    repository_root: &Path,
    manifest_path: &Path,
    reviewer: &str,
    replace_current: bool,
) -> Result<ApprovalRequest, OperationFailure> {
    let manifest: ReviewManifest = read_request(manifest_path, OPERATION)?;
    require_schema(
        OPERATION,
        &manifest.schema,
        REVIEW_MANIFEST_SCHEMA,
        &manifest.knowledge_id,
    )?;
    let foundation = load_operation_foundation(repository_root, OPERATION, &manifest.knowledge_id)?;
    if !foundation.owners.iter().any(|owner| owner.id == reviewer) {
        return Err(OperationFailure::new(
            OPERATION,
            "unknown_reviewer",
            format!("reviewer OwnerId `{reviewer}` does not exist"),
            vec![manifest.knowledge_id],
            "use a tracked OwnerId",
        ));
    }
    let replace_revision = if replace_current {
        let current_path = repository_root
            .join("methexis/approvals")
            .join(format!("{}.yaml", manifest.knowledge_id));
        let current = parse_approval(&current_path, repository_root).map_err(|diagnostic| {
            failure_from_diagnostic(
                OPERATION,
                diagnostic,
                "review the current approval record or drop --replace-current",
            )
        })?;
        Some(current.revision)
    } else {
        None
    };
    Ok(ApprovalRequest {
        schema: APPROVAL_REQUEST_SCHEMA.to_owned(),
        knowledge_id: manifest.knowledge_id,
        expected_revision: manifest.revision,
        projection_hash: manifest.projection_hash,
        reviewer: reviewer.to_owned(),
        reviewed_at: current_review_time(),
        replace_revision,
    })
}

fn current_review_time() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format_utc(seconds)
}

/// Renders epoch seconds as the `YYYY-MM-DDTHH:MM:SSZ` shape approval records
/// carry, using the proleptic Gregorian civil-from-days conversion.
fn format_utc(epoch_seconds: u64) -> String {
    let days = (epoch_seconds / 86_400) as i64;
    let seconds = epoch_seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // epoch 0과 알려진 2026-07-24T12:00:00Z가 승인 기록과 같은 UTC 형식으로 렌더링되는지 확인한다.
    #[test]
    fn epoch_seconds_render_the_approval_review_time_shape() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_784_894_400), "2026-07-24T12:00:00Z");
        assert_eq!(format_utc(86_399), "1970-01-01T23:59:59Z");
        assert!(crate::review::valid_review_time(&format_utc(1_784_894_400)));
    }
}
