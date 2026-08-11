use std::{collections::BTreeSet, path::Path};

use serde::Deserialize;

use super::model::CloseMetricsArtifact;
use crate::{bounded_file, review_protocol};

pub(super) const FILE_NAME: &str = "close-metrics.json";
const SCHEMA: &str = "yo.slice-close-metrics/v1";
const MAX_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metrics {
    schema: String,
    slice: String,
    slice_candidate: String,
    accepted_commit: String,
    execution_lanes: Vec<ExecutionLane>,
    review: Review,
    review_packets: ReviewPackets,
    validation: Vec<Validation>,
    elapsed_bottleneck: ElapsedBottleneck,
    known_unverified_environments: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionLane {
    lane: Lane,
    mode: Mode,
    operation_count: usize,
    max_concurrency: usize,
}

#[derive(Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum Lane {
    Discovery,
    Editing,
    Review,
    CargoValidation,
    Integration,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Parallel,
    Serial,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    rounds: usize,
    findings: Findings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Findings {
    reported: usize,
    resolved: usize,
    not_reproduced: usize,
    accepted_limits: usize,
    remaining: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewPackets {
    publication_count: usize,
    total_managed_tokens: usize,
    largest_sections: Vec<PacketSection>,
    reused_inputs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PacketSection {
    kind: String,
    name: String,
    rendered_bytes: usize,
    rendered_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Validation {
    name: String,
    argv: Vec<String>,
    runs: usize,
    status: ValidationStatus,
    reused: bool,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ValidationStatus {
    Passed,
    Unverified,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ElapsedBottleneck {
    name: String,
    elapsed_milliseconds: u64,
}

pub(super) fn capture(
    path: &Path,
    slice: &str,
    slice_candidate: &str,
    accepted_commit: &str,
) -> Result<CloseMetricsArtifact, String> {
    let bytes = bounded_file::read_regular(path, MAX_BYTES, "Slice close metrics")?;
    validate(&bytes, slice, slice_candidate, accepted_commit)?;
    Ok(CloseMetricsArtifact {
        path: path.to_path_buf(),
        hash: review_protocol::digest(&bytes),
    })
}

pub(super) fn require_current(
    artifact: &CloseMetricsArtifact,
    slice: &str,
    slice_candidate: &str,
    accepted_commit: &str,
) -> Result<(), String> {
    let current = capture(&artifact.path, slice, slice_candidate, accepted_commit)?;
    if current == *artifact {
        Ok(())
    } else {
        Err("Slice close metrics changed after planning".to_owned())
    }
}

fn validate(
    bytes: &[u8],
    slice: &str,
    slice_candidate: &str,
    accepted_commit: &str,
) -> Result<(), String> {
    let metrics: Metrics = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid Slice close metrics: {error}"))?;
    if metrics.schema != SCHEMA {
        return Err(format!("Slice close metrics must use schema `{SCHEMA}`"));
    }
    if metrics.slice != slice {
        return Err("Slice close metrics name does not match the planned Slice".to_owned());
    }
    review_protocol::require_commit(&metrics.slice_candidate, "Slice close metrics candidate")?;
    review_protocol::require_commit(
        &metrics.accepted_commit,
        "Slice close metrics accepted commit",
    )?;
    if metrics.slice_candidate != slice_candidate || metrics.accepted_commit != accepted_commit {
        return Err(
            "Slice close metrics do not identify the exact candidate and accepted commit"
                .to_owned(),
        );
    }
    validate_lanes(&metrics.execution_lanes)?;
    validate_review(&metrics.review)?;
    validate_packets(&metrics.review_packets)?;
    if metrics.review_packets.publication_count != 0 && metrics.review.rounds == 0 {
        return Err("published review packets require at least one review round".to_owned());
    }
    let has_unverified = validate_validation(&metrics.validation)?;
    validate_unique_nonblank(
        &metrics.known_unverified_environments,
        "known unverified environments",
    )?;
    if has_unverified != !metrics.known_unverified_environments.is_empty() {
        return Err(
            "unverified validation and known unverified environments must be recorded together"
                .to_owned(),
        );
    }
    if metrics.elapsed_bottleneck.name.trim().is_empty()
        || metrics.elapsed_bottleneck.elapsed_milliseconds == 0
    {
        return Err("Slice close metrics require a nonzero elapsed bottleneck".to_owned());
    }
    Ok(())
}

fn validate_lanes(lanes: &[ExecutionLane]) -> Result<(), String> {
    let mut kinds = BTreeSet::new();
    for lane in lanes {
        if !kinds.insert(lane.lane) || lane.operation_count == 0 || lane.max_concurrency == 0 {
            return Err(
                "Slice close execution lanes must be unique and have nonzero counts".to_owned(),
            );
        }
        match lane.mode {
            Mode::Serial if lane.max_concurrency != 1 => {
                return Err("serial Slice close lanes require max_concurrency 1".to_owned());
            },
            Mode::Parallel
                if lane.max_concurrency < 2 || lane.operation_count < lane.max_concurrency =>
            {
                return Err(
                    "parallel Slice close lanes require at least two concurrent operations"
                        .to_owned(),
                );
            },
            _ => {},
        }
        if matches!(lane.lane, Lane::CargoValidation | Lane::Integration)
            && lane.mode != Mode::Serial
        {
            return Err(
                "Cargo validation and integration must use serialized execution lanes".to_owned(),
            );
        }
    }
    if !kinds.contains(&Lane::Integration) {
        return Err("Slice close metrics require the integration execution lane".to_owned());
    }
    Ok(())
}

fn validate_review(review: &Review) -> Result<(), String> {
    let findings = &review.findings;
    let dispositions = [
        findings.resolved,
        findings.not_reproduced,
        findings.accepted_limits,
        findings.remaining,
    ]
    .into_iter()
    .try_fold(0usize, usize::checked_add)
    .ok_or_else(|| "Slice close finding totals exceed the supported range".to_owned())?;
    if dispositions != findings.reported {
        return Err("Slice close finding totals do not reconcile".to_owned());
    }
    if findings.remaining != 0 {
        return Err("accepted Slice close metrics cannot contain remaining findings".to_owned());
    }
    if review.rounds == 0 && findings.reported != 0 {
        return Err("Slice close findings require at least one review round".to_owned());
    }
    Ok(())
}

fn validate_packets(packets: &ReviewPackets) -> Result<(), String> {
    let valid_shape = if packets.publication_count == 0 {
        packets.total_managed_tokens == 0
            && packets.largest_sections.is_empty()
            && packets.reused_inputs.is_empty()
    } else {
        packets.total_managed_tokens != 0 && !packets.largest_sections.is_empty()
    };
    if !valid_shape {
        return Err(
            "Slice close packet totals must match whether packets were published".to_owned(),
        );
    }
    let mut sections = BTreeSet::new();
    for section in &packets.largest_sections {
        let identity = (section.kind.trim(), section.name.trim());
        if identity.0.is_empty()
            || identity.1.is_empty()
            || !sections.insert(identity)
            || section.rendered_bytes == 0
            || section.rendered_tokens == 0
        {
            return Err(
                "Slice close packet sections must be unique named nonzero measurements".to_owned(),
            );
        }
    }
    validate_unique_nonblank(&packets.reused_inputs, "reused packet inputs")
}

fn validate_validation(validation: &[Validation]) -> Result<bool, String> {
    if validation.is_empty() {
        return Err("Slice close metrics require validation commands".to_owned());
    }
    let mut names = BTreeSet::new();
    let mut has_unverified = false;
    for check in validation {
        let argv_valid = !check.argv.is_empty()
            && !check.argv[0].trim().is_empty()
            && check.argv.iter().all(|value| !value.contains('\0'));
        if check.name.trim().is_empty() || !names.insert(check.name.as_str()) || !argv_valid {
            return Err(
                "Slice close validation requires unique names and executable argv".to_owned(),
            );
        }
        match check.status {
            ValidationStatus::Passed if check.runs == 0 => {
                return Err("passed Slice close validation requires at least one run".to_owned());
            },
            ValidationStatus::Unverified if check.runs != 0 || check.reused => {
                return Err(
                    "unverified Slice close validation must have zero runs and cannot be reused"
                        .to_owned(),
                );
            },
            ValidationStatus::Unverified => has_unverified = true,
            ValidationStatus::Passed => {},
        }
    }
    Ok(has_unverified)
}

fn validate_unique_nonblank(values: &[String], label: &str) -> Result<(), String> {
    let mut unique = BTreeSet::new();
    if values
        .iter()
        .any(|value| value.trim().is_empty() || !unique.insert(value.as_str()))
    {
        Err(format!("{label} must be unique and nonblank"))
    } else {
        Ok(())
    }
}
