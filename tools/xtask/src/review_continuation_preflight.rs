use std::path::{Path, PathBuf};

use serde::Serialize;
use yo_core::{
    SessionId,
    session_repository::{
        LocalSessionReader, read_stored_session, read_stored_session_continuation,
    },
};

use crate::{
    bounded_file,
    review_egress::{self, AuthorizedDelivery},
    review_protocol::{digest, resolve_input_path},
};

mod delegated;
mod observation;
mod request;
mod validation;

pub(crate) use delegated::VerifiedHostContinuation;

const REQUEST_SCHEMA: &str = "yo.slice-review-continuation-preflight-request/v1alpha1";
const RESULT_SCHEMA: &str = "yo.slice-review-continuation-preflight-result/v1alpha1";
const REQUEST_LIMIT: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Serialize)]
struct Route<'a> {
    provider: &'a str,
    account: &'a str,
    model: &'a str,
}

#[derive(Debug, Serialize)]
struct ResultDocument<'a> {
    schema: &'static str,
    ok: bool,
    status: &'static str,
    next_action: &'static str,
    artifacts_published: bool,
    provider_requests: usize,
    request_id: String,
    egress_request_id: &'a str,
    review_id: &'a str,
    candidate_commit: &'a str,
    session_id: &'a str,
    route: Route<'a>,
    prior_packet_hash: &'a str,
    prior_provider_request_id: &'a str,
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
pub(crate) struct VerifiedContinuation {
    pub(crate) preflight_request_id: String,
    pub(crate) delivery: AuthorizedDelivery,
    pub(crate) session_root: PathBuf,
    pub(crate) continuation_anchor_sequence: u64,
    pub(crate) binding_epoch: u64,
}

pub(crate) fn evaluate_delegated(
    repository: &Path,
    request_path: &Path,
) -> Result<VerifiedHostContinuation, String> {
    delegated::evaluate(repository, request_path)
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let schema = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("invalid continuation preflight request: {error}"))?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "continuation preflight request has no string schema".to_owned())?
        .to_owned();
    if schema == delegated::REQUEST_SCHEMA {
        return delegated::run(repository, request_path);
    }
    let verified = evaluate(repository, request_path)?;
    let delivery = &verified.delivery;
    let (session_id, prior_packet_hash, prior_provider_request_id) =
        validation::require_finding_resolution(delivery)?;
    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        status: "eligible",
        next_action: "deliver_finding_resolution_once",
        artifacts_published: false,
        provider_requests: 0,
        request_id: verified.preflight_request_id,
        egress_request_id: &delivery.request_id,
        review_id: &delivery.review_id,
        candidate_commit: &delivery.candidate_commit,
        session_id,
        route: Route {
            provider: &delivery.provider,
            account: &delivery.account,
            model: &delivery.model,
        },
        prior_packet_hash,
        prior_provider_request_id,
        continuation_anchor_sequence: verified.continuation_anchor_sequence,
        binding_epoch: verified.binding_epoch,
    };
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode Slice review continuation preflight result: {error}")
        })?
    );
    Ok(())
}

pub(crate) fn evaluate(
    repository: &Path,
    request_path: &Path,
) -> Result<VerifiedContinuation, String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let request = request::parse_request(request_path, &request_bytes)?;
    let egress_request_path = resolve_input_path(repository, &request.egress_request_path);
    let egress_bytes = bounded_file::read_regular(
        &egress_request_path,
        REQUEST_LIMIT,
        "Slice review egress request",
    )?;
    if digest(&egress_bytes) != request.egress_request_hash {
        return Err("Slice review egress request hash does not match its frozen bytes".to_owned());
    }

    let delivery = review_egress::authorize_delivery(repository, &egress_request_path)?;
    let (session_id, prior_packet_hash, prior_provider_request_id) =
        validation::require_finding_resolution(&delivery)?;
    let session_id = session_id
        .parse::<SessionId>()
        .map_err(|error| format!("invalid reviewer Session identity: {error}"))?;
    let session_root =
        validation::resolve_session_root(repository, &request.session_repository_path)?;
    let reader = LocalSessionReader::open(&session_root)
        .map_err(|error| format!("cannot open reviewer Session repository: {error}"))?;
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot recover reviewer Session: {error}"))?;
    let observation = observation::observe_history(&history, &delivery)?;
    validation::validate_observation(&observation, prior_packet_hash, prior_provider_request_id)?;
    let continuation = read_stored_session_continuation(&reader, session_id).map_err(|error| {
        format!("reviewer Session is not eligible for finding-resolution continuation: {error}")
    })?;
    if continuation.target().session_id() != session_id {
        return Err("recovered continuation target differs from the authorized Session".to_owned());
    }
    let final_history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot revalidate reviewer Session: {error}"))?;
    let final_observation = observation::observe_history(&final_history, &delivery)?;
    if final_observation != observation {
        return Err("reviewer Session changed during continuation preflight".to_owned());
    }
    validation::require_unchanged_file(
        &egress_request_path,
        &egress_bytes,
        "Slice review egress request",
    )?;
    validation::require_unchanged_file(
        request_path,
        &request_bytes,
        "Slice review continuation preflight request",
    )?;

    Ok(VerifiedContinuation {
        preflight_request_id: digest(&request_bytes),
        delivery,
        session_root,
        continuation_anchor_sequence: continuation
            .target()
            .source_anchor_sequence()
            .ok_or_else(|| {
                "Slice review continuation requires a Continuation Anchor source".to_owned()
            })?
            .get(),
        binding_epoch: continuation.target().epoch(),
    })
}
