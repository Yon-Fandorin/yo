use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};

use super::{accepted_slice, metrics, standard_coordination_directory};
use crate::{bounded_file, review_protocol, slice_gate, slice_worktree};

const REQUEST_SCHEMA: &str = "yo.slice-close-prepare-request/v1alpha1";
const RESULT_SCHEMA: &str = "yo.slice-close-metrics-publication/v1alpha1";
const REQUEST_LIMIT: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    slice: String,
    gate_request_path: String,
    execution_lanes: Vec<ExecutionLane>,
    review: Review,
    review_packets: ReviewPackets,
    #[serde(default)]
    unverified_validation: Vec<UnverifiedValidation>,
    elapsed_bottleneck: ElapsedBottleneck,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionLane {
    lane: String,
    mode: String,
    operation_count: usize,
    max_concurrency: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Review {
    rounds: usize,
    findings: Findings,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Findings {
    reported: usize,
    resolved: usize,
    not_reproduced: usize,
    accepted_limits: usize,
    remaining: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewPackets {
    publication_count: usize,
    total_managed_tokens: usize,
    largest_sections: Vec<PacketSection>,
    reused_inputs: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PacketSection {
    kind: String,
    name: String,
    rendered_bytes: usize,
    rendered_tokens: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UnverifiedValidation {
    name: String,
    argv: Vec<String>,
    environment: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ElapsedBottleneck {
    name: String,
    elapsed_milliseconds: u64,
}

#[derive(Serialize)]
struct Metrics<'a> {
    schema: &'static str,
    slice: &'a str,
    slice_candidate: &'a str,
    accepted_commit: &'a str,
    execution_lanes: &'a [ExecutionLane],
    review: &'a Review,
    review_packets: &'a ReviewPackets,
    validation: Vec<Validation<'a>>,
    elapsed_bottleneck: &'a ElapsedBottleneck,
    known_unverified_environments: &'a [String],
}

#[derive(Serialize)]
struct Validation<'a> {
    name: &'a str,
    argv: &'a [String],
    runs: usize,
    status: &'static str,
    reused: bool,
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice close preparation request",
    )?;
    let request = parse_request(request_path, &request_bytes)?;
    let accepted = accepted_slice(repository, &request.slice)?;
    let workspace = slice_worktree::workspace_root(&accepted.repository)?;
    let gate_path = review_protocol::resolve_input_path(&workspace, &request.gate_request_path);
    let gate = slice_gate::ready(&accepted.worktree_path, &gate_path)?;
    let bytes = build_metrics(
        &request,
        &gate,
        &accepted.slice_head,
        &accepted.accepted_commit,
    )?;

    let current_request = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice close preparation request",
    )?;
    if current_request != request_bytes {
        return Err("Slice close preparation request changed before publication".to_owned());
    }
    let current = accepted_slice(repository, &request.slice)?;
    let current_gate = slice_gate::ready(&current.worktree_path, &gate_path)?;
    if current.integration_head != accepted.integration_head
        || current.slice_head != accepted.slice_head
        || current.accepted_commit != accepted.accepted_commit
        || current_gate != gate
    {
        return Err("post-gate close inputs changed before publication".to_owned());
    }

    let output = standard_coordination_directory(&accepted.repository, &request.slice)?
        .join(metrics::FILE_NAME);
    let created = bounded_file::publish_new_or_exact(
        &output,
        &bytes,
        metrics::MAX_BYTES,
        "prepared Slice close metrics",
    )?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": RESULT_SCHEMA,
            "ok": true,
            "status": if created { "written" } else { "reused" },
            "slice": request.slice,
            "slice_candidate": accepted.slice_head,
            "accepted_commit": accepted.accepted_commit,
            "metrics_path": output,
            "metrics_hash": review_protocol::digest(&bytes)
        }))
        .map_err(|error| format!("cannot encode Slice close metrics publication: {error}"))?
    );
    Ok(())
}

fn parse_request(path: &Path, bytes: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "invalid Slice close preparation request {}: {error}",
            path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice close preparation schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    Ok(request)
}

fn build_metrics(
    request: &Request,
    gate: &slice_gate::ReadyGate,
    slice_candidate: &str,
    accepted_commit: &str,
) -> Result<Vec<u8>, String> {
    if gate.slice != request.slice || gate.candidate_commit != slice_candidate {
        return Err("ready gate does not identify the accepted Slice candidate".to_owned());
    }
    if gate.review_count == 0 && request.review.rounds != 0
        || gate.review_count != 0 && request.review.rounds == 0
    {
        return Err(
            "review rounds must match whether the ready gate contains review evidence".to_owned(),
        );
    }
    let gate_environments = gate
        .known_unverified_environments
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let observed_environments = request
        .unverified_validation
        .iter()
        .map(|entry| entry.environment.as_str())
        .collect::<BTreeSet<_>>();
    if gate_environments != observed_environments
        || gate_environments.len() != request.unverified_validation.len()
    {
        return Err(
            "unverified validation must map one-to-one to the ready gate environments".to_owned(),
        );
    }

    let mut validation = gate
        .validation
        .iter()
        .map(|entry| Validation {
            name: &entry.name,
            argv: &entry.argv,
            runs: 1,
            status: "passed",
            reused: entry.reused,
        })
        .collect::<Vec<_>>();
    validation.extend(
        request
            .unverified_validation
            .iter()
            .map(|entry| Validation {
                name: &entry.name,
                argv: &entry.argv,
                runs: 0,
                status: "unverified",
                reused: false,
            }),
    );
    let metrics = Metrics {
        schema: metrics::SCHEMA,
        slice: &request.slice,
        slice_candidate,
        accepted_commit,
        execution_lanes: &request.execution_lanes,
        review: &request.review,
        review_packets: &request.review_packets,
        validation,
        elapsed_bottleneck: &request.elapsed_bottleneck,
        known_unverified_environments: &gate.known_unverified_environments,
    };
    let mut bytes = serde_json::to_vec_pretty(&metrics)
        .map_err(|error| format!("cannot encode prepared Slice close metrics: {error}"))?;
    bytes.push(b'\n');
    metrics::validate(&bytes, &request.slice, slice_candidate, accepted_commit)?;
    Ok(bytes)
}

#[cfg(test)]
pub(super) fn build_metrics_for_test(
    request: &[u8],
    gate: &slice_gate::ReadyGate,
    slice_candidate: &str,
    accepted_commit: &str,
) -> Result<Vec<u8>, String> {
    let request = parse_request(Path::new("request.json"), request)?;
    build_metrics(&request, gate, slice_candidate, accepted_commit)
}
