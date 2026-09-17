use std::path::Path;

use super::model::{CapturedInput, CapturedSource, Report, Request};
use crate::{bounded_file, review_protocol};

const REPORT_SCHEMA: &str = "yo.slice-cost-report/v1alpha1";
pub(super) const POLICY: &str = "owners-separated/no-cross-owner-total/v1alpha1";
const REPORT_LIMIT: usize = 512 * 1024;

pub(super) fn render(
    request: &Request,
    request_path: &Path,
    request_bytes: &[u8],
    sources: Vec<CapturedSource>,
) -> Result<Vec<u8>, String> {
    let report = Report {
        schema: REPORT_SCHEMA,
        aggregation_policy: POLICY,
        slice: &request.slice,
        candidate_commit: &request.candidate_commit,
        request: CapturedInput {
            path: request_path,
            hash: review_protocol::digest(request_bytes),
            bytes: request_bytes.len(),
        },
        source_artifacts: sources,
        owners: &request.owners,
    };
    let mut report_bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("cannot encode Slice cost report: {error}"))?;
    report_bytes.push(b'\n');
    if report_bytes.len() > REPORT_LIMIT {
        return Err("Slice cost report exceeds its output limit".to_owned());
    }
    Ok(report_bytes)
}

pub(super) fn publish(output: &Path, report: &[u8]) -> Result<bool, String> {
    bounded_file::publish_new_or_exact(output, report, REPORT_LIMIT, "Slice cost report")
}

pub(super) fn publication(output: &Path, report: &[u8], created: bool) -> Result<String, String> {
    serde_json::to_string(&serde_json::json!({
        "schema": "yo.slice-cost-report-publication/v1alpha1",
        "ok": true,
        "status": if created { "written" } else { "reused" },
        "report_path": output,
        "report_hash": review_protocol::digest(report)
    }))
    .map_err(|error| format!("cannot encode Slice cost publication: {error}"))
}
