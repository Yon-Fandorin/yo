use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use yo_core::{
    AgentCommand, SessionId, TranscriptRecord,
    session_repository::{
        LocalSessionReader, StoredRequestTraceRecord, read_stored_session,
        read_stored_session_continuation,
    },
};

use super::{
    REQUEST_LIMIT, bounded_file, compact_path, digest, require_sha256, require_unchanged_file,
    resolve_input_path, resolve_session_root,
};
use crate::{
    review_egress::{self, AuthorizedHostDelivery},
    review_session::{
        delegated_backend_kind_matches, delegated_binding_matches, host_request_identity,
    },
};

pub(super) const REQUEST_SCHEMA: &str =
    "yo.slice-review-delegated-continuation-preflight-request/v1alpha1";
const RESULT_SCHEMA: &str = "yo.slice-review-delegated-continuation-preflight-result/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    egress_request_path: String,
    egress_request_hash: String,
    session_repository_path: String,
}

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
        require_finding_resolution(delivery)?;
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
    let request = parse_request(request_path, &request_bytes)?;
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
        require_finding_resolution(&delivery)?;
    let session_id = session_id
        .parse::<SessionId>()
        .map_err(|error| format!("invalid delegated reviewer Session identity: {error}"))?;
    let session_root = resolve_session_root(repository, &request.session_repository_path)?;
    let reader = LocalSessionReader::open(&session_root)
        .map_err(|error| format!("cannot open delegated reviewer Session repository: {error}"))?;
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot recover delegated reviewer Session: {error}"))?;
    let observation = observe_history(&history, &delivery)?;
    validate_observation(&observation, prior_packet_hash, prior_host_request_id)?;
    let continuation = read_stored_session_continuation(&reader, session_id).map_err(|error| {
        format!("delegated reviewer Session is not eligible for continuation: {error}")
    })?;
    if continuation.target().session_id() != session_id {
        return Err("recovered continuation target differs from the authorized Session".to_owned());
    }
    let final_history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot revalidate delegated reviewer Session: {error}"))?;
    if observe_history(&final_history, &delivery)? != observation {
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

fn parse_request(path: &Path, bytes: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "invalid delegated continuation preflight request {}: {error}",
            path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported delegated continuation preflight schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.egress_request_path, "egress_request_path")?;
    compact_path(&request.session_repository_path, "session_repository_path")?;
    require_sha256(&request.egress_request_hash, "egress_request_hash")?;
    Ok(request)
}

fn require_finding_resolution(
    delivery: &AuthorizedHostDelivery,
) -> Result<(&str, &str, &str), String> {
    if delivery.review_kind != "finding_resolution" || delivery.fresh_session {
        return Err(
            "delegated continuation preflight accepts only an authorized finding-resolution resume"
                .to_owned(),
        );
    }
    let session_id = delivery
        .session_id
        .as_deref()
        .ok_or_else(|| "delegated finding-resolution has no reviewer Session".to_owned())?;
    let packet_hash = delivery
        .prior_packet_hash
        .as_deref()
        .ok_or_else(|| "delegated finding-resolution has no prior packet hash".to_owned())?;
    let request_id = delivery.prior_host_request_id.as_deref().ok_or_else(|| {
        "delegated finding-resolution has no prior host request identity".to_owned()
    })?;
    Ok((session_id, packet_hash, request_id))
}

fn observe_history(
    history: &yo_core::session_repository::StoredSessionHistory,
    delivery: &AuthorizedHostDelivery,
) -> Result<Observation, String> {
    let start_packet_hashes = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { input, .. }) => {
                Some(digest(input.as_str().as_bytes()))
            },
            _ => None,
        })
        .collect();
    let mut binding_matches = Vec::new();
    let mut request_identities = Vec::new();
    let mut outcome_identities = Vec::new();
    let mut continuation_anchors = Vec::new();
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::BindingOpened {
                backend_kind,
                binding_identity,
                ..
            } => binding_matches.push(
                delegated_backend_kind_matches(backend_kind, &delivery.host)
                    && delegated_binding_matches(
                        binding_identity.schema(),
                        binding_identity.value(),
                        &delivery.host,
                        &delivery.execution_profile,
                    )?,
            ),
            StoredRequestTraceRecord::RequestAccepted {
                request_identity, ..
            } => request_identities.push(request_identity.value().to_owned()),
            StoredRequestTraceRecord::ResumableOutcome {
                outcome_identity, ..
            } => outcome_identities.push(
                outcome_identity
                    .as_ref()
                    .map(|identity| identity.value().to_owned()),
            ),
            StoredRequestTraceRecord::ContinuationAnchor {
                epoch,
                accepted_request_sequence,
                resumable_outcome_sequence,
                journal_boundary,
            } => continuation_anchors.push((
                *epoch,
                accepted_request_sequence.get(),
                resumable_outcome_sequence.get(),
                journal_boundary.get(),
            )),
            _ => {},
        }
    }
    Ok(Observation {
        start_packet_hashes,
        binding_matches,
        request_identities,
        outcome_identities,
        continuation_anchors,
    })
}

fn validate_observation(
    observation: &Observation,
    expected_packet_hash: &str,
    expected_host_request_id: &str,
) -> Result<(), String> {
    if observation.start_packet_hashes != [expected_packet_hash] {
        return Err(
            "delegated reviewer Session does not contain exactly one matching original StartTurn"
                .to_owned(),
        );
    }
    if observation.binding_matches != [true] {
        return Err(
            "delegated reviewer Session does not contain exactly one matching host review binding"
                .to_owned(),
        );
    }
    let observed = host_request_identity(
        &observation.request_identities,
        &observation.outcome_identities,
    )?;
    if observed != expected_host_request_id {
        return Err(
            "delegated reviewer Session changed the prior host request identity".to_owned(),
        );
    }
    if observation.continuation_anchors.len() != 1 {
        return Err(
            "delegated reviewer Session must contain exactly one Continuation Anchor".to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // delegated continuation preflight는 managed schema나 Provider 좌표를 받아들이지 않고
    // exact egress bytes와 Session repository만 가리키는 closed alpha shape를 유지합니다.
    #[test]
    fn request_has_a_closed_delegated_alpha_shape() {
        let value = serde_json::json!({
            "schema": REQUEST_SCHEMA,
            "egress_request_path": ".local-exclude/egress.json",
            "egress_request_hash": format!("sha256:{}", "a".repeat(64)),
            "session_repository_path": "/tmp/sessions"
        });
        parse_request(
            Path::new("request.json"),
            &serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let mut fabricated = value;
        fabricated["provider"] = "codex".into();
        assert!(
            parse_request(
                Path::new("request.json"),
                &serde_json::to_vec(&fabricated).unwrap()
            )
            .unwrap_err()
            .contains("unknown field")
        );
    }
}
