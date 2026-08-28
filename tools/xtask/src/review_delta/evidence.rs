use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    AffectedPathPolicy, MAX_AGGREGATE_EVIDENCE_BYTES, MAX_INPUT_BYTES,
    capture::{capture_file, captured, require_exact_hash},
    model::{
        EvidenceRequest, FindingDisposition, NamedArtifact, PRIOR_FINDINGS_SCHEMA, PriorFindings,
        Request,
    },
};
use crate::{
    bounded_file,
    review_packet::VerifiedReview,
    review_protocol::{
        Captured, NamedCaptured, artifact, require_commit, resolve_input_path, sorted_unique,
    },
    validation_summary,
};

pub(super) fn validate_transition(
    context: TransitionContext<'_>,
    prior: &VerifiedReview,
    replacement_candidate: &str,
    delta: &Captured,
    findings: &[FindingDisposition],
    reused: &[NamedCaptured],
    affected: &[NamedCaptured],
) -> Result<(), String> {
    require_commit(replacement_candidate, "replacement candidate")?;
    if delta.bytes.is_empty() {
        return Err("replacement candidate has no delta from the prior candidate".to_owned());
    }
    if sorted_findings(findings)? != findings {
        return Err("finding dispositions are not in canonical finding-ID order".to_owned());
    }
    if affected.is_empty() {
        return Err("published review delta has no replacement-specific evidence".to_owned());
    }
    let prior_by_name = prior
        .validation_evidence
        .iter()
        .map(|evidence| (evidence.name.as_str(), evidence))
        .collect::<BTreeMap<_, _>>();
    let reused_by_name = reused
        .iter()
        .map(|evidence| (evidence.name.as_str(), evidence))
        .collect::<BTreeMap<_, _>>();
    let affected_names = affected
        .iter()
        .map(|evidence| evidence.name.as_str())
        .collect::<BTreeSet<_>>();
    if reused_by_name
        .keys()
        .any(|name| affected_names.contains(name))
    {
        return Err("published validation evidence is both reused and affected".to_owned());
    }
    for (name, reused) in &reused_by_name {
        let expected = prior_by_name
            .get(name)
            .ok_or_else(|| format!("unknown reused validation evidence `{name}`"))?;
        if reused.artifact.path != expected.path || reused.artifact.hash != expected.hash {
            return Err(format!(
                "published reused validation evidence `{}` changed",
                expected.name
            ));
        }
    }
    for evidence in &prior.validation_evidence {
        if !reused_by_name.contains_key(evidence.name.as_str())
            && !affected_names.contains(evidence.name.as_str())
        {
            return Err("published review delta omits prior validation evidence".to_owned());
        }
    }
    for evidence in affected {
        let previous = prior_by_name.get(evidence.name.as_str());
        match context.affected_path_policy {
            AffectedPathPolicy::LegacyStringIdentity => {
                crate::review_packet::external_operation::validate(
                    &evidence.name,
                    &evidence.artifact.bytes,
                    replacement_candidate,
                )?;
                if previous.is_some_and(|previous| {
                    previous.path == evidence.artifact.path
                        && previous.hash == evidence.artifact.hash
                }) {
                    return Err(format!(
                        "affected validation evidence `{}` is unchanged from the prior candidate",
                        evidence.name
                    ));
                }
            },
            AffectedPathPolicy::CanonicalIdentity => {
                if let Some(previous) = previous {
                    require_new_affected_path(
                        context.repository,
                        &evidence.name,
                        &previous.path,
                        &evidence.artifact.path,
                    )?;
                }
                crate::review_packet::external_operation::validate(
                    &evidence.name,
                    &evidence.artifact.bytes,
                    replacement_candidate,
                )?;
            },
        }
        if !evidence
            .artifact
            .bytes
            .windows(replacement_candidate.len())
            .any(|window| window == replacement_candidate.as_bytes())
        {
            return Err(format!(
                "affected validation evidence `{}` does not bind the replacement candidate commit",
                evidence.name
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct TransitionContext<'a> {
    repository: &'a Path,
    affected_path_policy: AffectedPathPolicy,
}

impl<'a> TransitionContext<'a> {
    pub(super) fn new(repository: &'a Path, affected_path_policy: AffectedPathPolicy) -> Self {
        Self {
            repository,
            affected_path_policy,
        }
    }
}

fn require_new_affected_path(
    repository: &Path,
    name: &str,
    previous: &str,
    affected: &str,
) -> Result<(), String> {
    let canonical = |value: &str| {
        let path = resolve_input_path(repository, value);
        std::fs::canonicalize(&path).map_err(|error| {
            format!(
                "cannot resolve validation evidence path {}: {error}",
                path.display()
            )
        })
    };
    if canonical(previous)? == canonical(affected)? {
        Err(format!(
            "affected validation evidence `{name}` must use a new immutable path"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn validate_findings_artifact(
    captured: &Captured,
    prior: &VerifiedReview,
) -> Result<(), String> {
    let findings: PriorFindings = serde_json::from_slice(&captured.bytes)
        .map_err(|error| format!("invalid prior review findings: {error}"))?;
    let mut ids = BTreeSet::new();
    if findings.schema != PRIOR_FINDINGS_SCHEMA
        || findings.review_id != prior.review_id
        || findings.candidate_commit != prior.candidate_commit
        || findings.findings.is_empty()
        || findings.findings.iter().any(|finding| {
            finding.finding_id.trim().is_empty()
                || finding.summary.trim().is_empty()
                || !ids.insert(finding.finding_id.clone())
        })
    {
        return Err(
            "prior findings do not exactly identify a valid prior review result".to_owned(),
        );
    }
    Ok(())
}

pub(super) fn capture_named_artifacts(
    repository: &Path,
    records: &[NamedArtifact],
    label: &str,
) -> Result<Vec<NamedCaptured>, String> {
    let mut names = BTreeSet::new();
    let mut values = Vec::new();
    for record in records {
        if record.name.trim().is_empty() || !names.insert(record.name.clone()) {
            return Err(format!("{label} names must be non-empty and unique"));
        }
        let artifact_value = capture_file(
            &resolve_input_path(repository, &record.artifact.path),
            label,
        )?;
        if artifact(&artifact_value) != record.artifact {
            return Err(format!("{label} `{}` changed", record.name));
        }
        values.push(NamedCaptured {
            name: record.name.clone(),
            artifact: artifact_value,
        });
    }
    values.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(values)
}

pub(super) fn capture_prior_findings(
    repository: &Path,
    request: &Request,
    prior: &VerifiedReview,
) -> Result<Captured, String> {
    let path = resolve_input_path(repository, &request.prior_findings_path);
    let bytes = bounded_file::read_regular(&path, MAX_INPUT_BYTES, "prior review findings")?;
    require_exact_hash(
        &request.prior_findings_hash,
        &bytes,
        "prior review findings",
    )?;
    let captured = captured(path.to_string_lossy().into_owned(), bytes)?;
    validate_findings_artifact(&captured, prior)?;
    Ok(captured)
}

pub(super) fn capture_validation(
    repository: &Path,
    prior: &VerifiedReview,
    replacement_candidate: &str,
    verify_affected_identity: bool,
    reused_names: &[String],
    affected_requests: &[EvidenceRequest],
) -> Result<(Vec<NamedCaptured>, Vec<NamedCaptured>), String> {
    let reused_names = sorted_unique(reused_names, "reused validation evidence name")?;
    let reused_set = reused_names.iter().cloned().collect::<BTreeSet<_>>();
    let mut affected_names = BTreeSet::new();
    let mut affected = Vec::new();
    let mut aggregate_bytes = 0usize;
    for request in affected_requests {
        if request.name.trim().is_empty() || !affected_names.insert(request.name.clone()) {
            return Err(
                "affected validation evidence names must be non-empty and unique".to_owned(),
            );
        }
        if reused_set.contains(&request.name) {
            return Err("validation evidence cannot be both reused and affected".to_owned());
        }
        let artifact = capture_file(
            &resolve_input_path(repository, &request.path),
            "affected validation evidence",
        )?;
        if verify_affected_identity {
            validation_summary::verify_review_input(
                repository,
                &artifact.bytes,
                &request.name,
                replacement_candidate,
            )
            .map_err(|error| {
                format!(
                    "invalid affected validation evidence for `{}`: {error}",
                    request.name
                )
            })?;
        }
        let item = NamedCaptured {
            name: request.name.clone(),
            artifact,
        };
        add_evidence_size(&mut aggregate_bytes, &item)?;
        affected.push(item);
    }
    affected.sort_by(|left, right| left.name.cmp(&right.name));

    let prior_by_name = prior
        .validation_evidence
        .iter()
        .map(|evidence| (evidence.name.as_str(), evidence))
        .collect::<BTreeMap<_, _>>();
    if prior_by_name
        .keys()
        .any(|name| !reused_set.contains(*name) && !affected_names.contains(*name))
    {
        return Err(
            "every prior validation item must be classified as reused or affected".to_owned(),
        );
    }
    let mut reused = Vec::new();
    for name in reused_names {
        let expected = prior_by_name
            .get(name.as_str())
            .ok_or_else(|| format!("unknown reused validation evidence `{name}`"))?;
        let artifact = capture_file(
            &resolve_input_path(repository, &expected.path),
            "reused validation evidence",
        )?;
        if artifact.hash != expected.hash {
            return Err(format!("reused validation evidence `{name}` changed"));
        }
        let item = NamedCaptured { name, artifact };
        add_evidence_size(&mut aggregate_bytes, &item)?;
        reused.push(item);
    }
    Ok((reused, affected))
}

pub(super) fn add_evidence_size(total: &mut usize, evidence: &NamedCaptured) -> Result<(), String> {
    add_evidence_bytes(total, evidence.artifact.bytes.len())
}

pub(super) fn add_evidence_bytes(total: &mut usize, bytes: usize) -> Result<(), String> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| "aggregate validation evidence size overflowed".to_owned())?;
    if *total > MAX_AGGREGATE_EVIDENCE_BYTES {
        Err(format!(
            "aggregate validation evidence exceeds the {MAX_AGGREGATE_EVIDENCE_BYTES}-byte limit"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn sorted_findings(
    values: &[FindingDisposition],
) -> Result<Vec<FindingDisposition>, String> {
    let mut sorted = values.to_vec();
    if sorted
        .iter()
        .any(|finding| finding.finding_id.trim().is_empty() || finding.summary.trim().is_empty())
    {
        return Err("finding IDs and summaries must not be blank".to_owned());
    }
    sorted.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    if sorted
        .windows(2)
        .any(|pair| pair[0].finding_id == pair[1].finding_id)
    {
        return Err("finding IDs must be unique".to_owned());
    }
    Ok(sorted)
}

pub(super) fn require_exact_finding_set(
    prior_findings: &Captured,
    dispositions: &[FindingDisposition],
) -> Result<(), String> {
    let prior: PriorFindings = serde_json::from_slice(&prior_findings.bytes)
        .map_err(|error| format!("invalid prior review findings: {error}"))?;
    let expected = prior
        .findings
        .into_iter()
        .map(|finding| finding.finding_id)
        .collect::<BTreeSet<_>>();
    let actual = dispositions
        .iter()
        .map(|finding| finding.finding_id.clone())
        .collect::<BTreeSet<_>>();
    if expected == actual && actual.len() == dispositions.len() {
        Ok(())
    } else {
        Err("finding dispositions must reconcile the exact prior finding ID set".to_owned())
    }
}
