mod observation;
mod publish;
mod request;

use std::path::Path;

use super::{
    REQUEST_LIMIT, admission::read_request, artifact::require_exact_file_hash,
    model::DeliveryRequest,
};
use crate::bounded_file;

const RESULT_SCHEMA: &str = "yo.slice-review-delegated-delivery-finalize-result/v1alpha1";

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let loaded = request::read(repository, request_path)?;
    let finalized = match read_request(&loaded.delivery_request_path)? {
        DeliveryRequest::Delegated(request) => publish::finalize_original(repository, request)?,
        DeliveryRequest::DelegatedContinuation(request) => {
            publish::finalize_continuation(repository, request)?
        },
        _ => {
            return Err(
                "delivery finalization accepts only a delegated original or continuation request"
                    .to_owned(),
            );
        },
    };

    require_exact_file_hash(
        &loaded.delivery_request_path,
        &loaded.request.delivery_request_hash,
        REQUEST_LIMIT,
        "delegated delivery request",
    )?;
    if bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "delegated delivery finalization request",
    )? != loaded.bytes
    {
        return Err("delegated delivery finalization request changed during recovery".to_owned());
    }
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": RESULT_SCHEMA,
            "ok": true,
            "status": if finalized.created { "finalized" } else { "reused" },
            "next_action": "interpret_review",
            "request_id": finalized.request_id,
            "review_id": finalized.review_id,
            "candidate_commit": finalized.candidate_commit,
            "session_id": finalized.session_id,
            "host_request_id": finalized.host_request_id,
            "delivery_receipt_path": finalized.delivery_path,
            "delivery_receipt_hash": finalized.delivery_hash,
            "provider_requests": 0,
            "host_requests": 0
        }))
        .map_err(|error| format!("cannot encode delegated finalization result: {error}"))?
    );
    Ok(())
}
