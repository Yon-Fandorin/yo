mod delegated;
mod model;
mod original;
mod validation;

#[cfg(test)]
mod tests;

use std::path::Path;

use model::{DELEGATED_REQUEST_SCHEMA, DELIVERY_RECEIPT_SCHEMA};
use validation::{DELIVERY_RECEIPT_LIMIT, REQUEST_LIMIT};

use crate::{bounded_file, review_packet, review_protocol::resolve_input_path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VerifiedDeliveryRoute {
    Managed {
        provider: String,
        model: String,
        session_id: String,
    },
    Delegated {
        host: String,
        session_id: String,
    },
}

pub(crate) use delegated::{AuthorizedHostDelivery, authorize_host_delivery};
pub(crate) use original::{AuthorizedDelivery, authorize_delivery};

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice review egress request")?;
    let schema = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("invalid Slice review egress request: {error}"))?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Slice review egress request has no string schema".to_owned())?
        .to_owned();
    if schema == DELEGATED_REQUEST_SCHEMA {
        delegated::run(repository, request_path)
    } else {
        original::run(repository, request_path)
    }
}

pub(crate) fn verify_completed_delivery(
    repository: &Path,
    receipt_path: &Path,
    review: &review_packet::VerifiedReview,
) -> Result<VerifiedDeliveryRoute, String> {
    original::verify_completed_delivery(repository, receipt_path, review)
}

pub(crate) fn verify_any_completed_delivery(
    repository: &Path,
    receipt_path: &Path,
    review: &review_packet::VerifiedReview,
) -> Result<VerifiedDeliveryRoute, String> {
    let path = resolve_input_path(repository, &receipt_path.to_string_lossy());
    let bytes = bounded_file::read_regular(
        &path,
        DELIVERY_RECEIPT_LIMIT,
        "external review delivery receipt",
    )?;
    let schema = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("invalid external review delivery receipt: {error}"))?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "external review delivery receipt has no string schema".to_owned())?
        .to_owned();
    if schema == DELIVERY_RECEIPT_SCHEMA {
        verify_completed_delivery(repository, receipt_path, review)
    } else {
        delegated::verify_completed_delivery_bytes(&bytes, review)
    }
}
