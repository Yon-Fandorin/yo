use std::path::{Path, PathBuf};

use serde::Serialize;
use yo_core::{
    SessionId,
    session_repository::{
        LocalSessionReader, read_stored_session, read_stored_session_continuation,
    },
};

use super::{
    REQUEST_LIMIT, bounded_file, digest, resolve_input_path,
    validation::{require_unchanged_file, resolve_session_root},
};
use crate::review_egress::{self, AuthorizedHostDelivery};

mod observation;
mod request;

pub(super) const REQUEST_SCHEMA: &str =
    "yo.slice-review-delegated-continuation-preflight-request/v1alpha1";

const RESULT_SCHEMA: &str = "yo.slice-review-delegated-continuation-preflight-result/v1alpha1";

#[derive(Debug, Serialize)]
struct Target<'a> {
    kind: &'static str,
    host: &'a str,
}

#[derive(Debug, Serialize)]
struct ResultDocument<'a> {
    schema: &'static str,
    ok: bool,
    status: &'static str,
    next_action: &'static str,
    artifacts_published: bool,
    host_requests: usize,
    request_id: String,
    egress_request_id: &'a str,
    review_id: &'a str,
    candidate_commit: &'a str,
    session_id: &'a str,
    target: Target<'a>,
    execution_profile: &'a str,
    prior_packet_hash: &'a str,
    prior_host_request_id: &'a str,
    continuation_anchor_sequence: u64,
    binding_epoch: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct Observation {
    start_packet_hashes: Vec<String>,
    binding_matches: Vec<bool>,
    request_identities: Vec<String>,
    outcome_identities: Vec<Option<String>>,
    continuation_anchors: Vec<(u64, u64, u64, u64)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedHostContinuation {
    pub(crate) preflight_request_id: String,
    pub(crate) delivery: AuthorizedHostDelivery,
    pub(crate) session_root: PathBuf,
    pub(crate) continuation_anchor_sequence: u64,
    pub(crate) binding_epoch: u64,
}

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let verified = evaluate(repository, request_path)?;
    let delivery = &verified.delivery;
    let (session_id, prior_packet_hash, prior_host_request_id) =
        request::require_finding_resolution(delivery)?;
    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        status: "eligible",
        next_action: "deliver_delegated_finding_resolution_once",
        artifacts_published: false,
        host_requests: 0,
        request_id: verified.preflight_request_id,
        egress_request_id: &delivery.request_id,
        review_id: &delivery.review_id,
        candidate_commit: &delivery.candidate_commit,
        session_id,
        target: Target {
            kind: "delegated_host",
            host: &delivery.host,
        },
        execution_profile: &delivery.execution_profile,
        prior_packet_hash,
        prior_host_request_id,
        continuation_anchor_sequence: verified.continuation_anchor_sequence,
        binding_epoch: verified.binding_epoch,
    };
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode delegated continuation preflight result: {error}")
        })?
    );
    Ok(())
}

pub(crate) fn evaluate(
    repository: &Path,
    request_path: &Path,
) -> Result<VerifiedHostContinuation, String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "delegated Slice review continuation preflight request",
    )?;
    let request = request::parse_request(request_path, &request_bytes)?;
    let egress_request_path = resolve_input_path(repository, &request.egress_request_path);
    let egress_bytes = bounded_file::read_regular(
        &egress_request_path,
        REQUEST_LIMIT,
        "delegated Slice review egress request",
    )?;
    if digest(&egress_bytes) != request.egress_request_hash {
        return Err("delegated egress request hash does not match its frozen bytes".to_owned());
    }

    let delivery = review_egress::authorize_host_delivery(repository, &egress_request_path)?;
    let (session_id, prior_packet_hash, prior_host_request_id) =
        request::require_finding_resolution(&delivery)?;
    let session_id = session_id
        .parse::<SessionId>()
        .map_err(|error| format!("invalid delegated reviewer Session identity: {error}"))?;
    let session_root = resolve_session_root(repository, &request.session_repository_path)?;
    let reader = LocalSessionReader::open(&session_root)
        .map_err(|error| format!("cannot open delegated reviewer Session repository: {error}"))?;
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot recover delegated reviewer Session: {error}"))?;
    let observation = observation::observe_history(&history, &delivery)?;
    observation::validate_observation(&observation, prior_packet_hash, prior_host_request_id)?;
    let continuation = read_stored_session_continuation(&reader, session_id).map_err(|error| {
        format!("delegated reviewer Session is not eligible for continuation: {error}")
    })?;
    if continuation.target().session_id() != session_id {
        return Err("recovered continuation target differs from the authorized Session".to_owned());
    }
    let final_history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot revalidate delegated reviewer Session: {error}"))?;
    if observation::observe_history(&final_history, &delivery)? != observation {
        return Err("delegated reviewer Session changed during continuation preflight".to_owned());
    }
    require_unchanged_file(
        &egress_request_path,
        &egress_bytes,
        "delegated Slice review egress request",
    )?;
    require_unchanged_file(
        request_path,
        &request_bytes,
        "delegated continuation preflight request",
    )?;
    Ok(VerifiedHostContinuation {
        preflight_request_id: digest(&request_bytes),
        delivery,
        session_root,
        continuation_anchor_sequence: continuation
            .target()
            .source_anchor_sequence()
            .ok_or_else(|| {
                "delegated review continuation requires a Continuation Anchor source".to_owned()
            })?
            .get(),
        binding_epoch: continuation.target().epoch(),
    })
}
