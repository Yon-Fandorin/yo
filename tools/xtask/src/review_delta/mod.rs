mod model;

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};

use serde::Serialize;

use self::model::{
    Artifact, ArtifactWithTokens, DELIVERY_PROFILE, DeliveryProfile, EvidenceRequest,
    FindingDisposition, MANIFEST_SCHEMA, Manifest, ManifestInputs, NamedArtifact,
    NamedSemanticInput, PLAN_SCHEMA, PRIOR_FINDINGS_SCHEMA, PacketRecord, PriorFindings,
    REQUEST_SCHEMA, RESULT_SCHEMA, Request, ResultRecord, ReviewDeltaPlan, TOKENIZER_COMPILER,
    TOKENIZER_PROFILE,
};
use crate::{
    bounded_file, git,
    review_packet::{self, VerifiedReview, storage},
    review_protocol::{
        Captured, NamedCaptured, artifact, digest, domain_digest, relative, require_commit,
        resolve_input_path, sorted_unique,
    },
    slice_contract,
};

const REVIEW_DELTA_ID_DOMAIN: &[u8] = b"yo.slice-review-delta/v1";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
const MAX_AGGREGATE_EVIDENCE_BYTES: usize = 32 * 1024 * 1024;
const PREAMBLE: &str = "# yo Slice Finding-Resolution Review Delta\n\nThis is the complete caller-controlled continuation payload for the same reviewer, lens, and scope. Identify the prior ReviewId and candidate from your existing session, verify that the supplied prior-findings artifact is the exact finding set you issued, then inspect only the replacement delta and supplied dispositions/evidence. Fail closed if that prior context or exact finding set is unavailable.\n";
const SECTION_PREFIX: &str = "\n<<<YO-REVIEW-DELTA-SECTION ";
const METADATA_SUFFIX: &str = ">>>\n";
const SECTION_SUFFIX: &str = "\n<<<YO-REVIEW-DELTA-SECTION-END>>>\n";
const PAYLOAD_SUFFIX: &str = "\n<<<YO-REVIEW-DELTA-PAYLOAD-END>>>\n";

struct Inputs {
    request: Captured,
    prior_manifest: Captured,
    prior_packet: Captured,
    prior_findings: Captured,
    prior: VerifiedReview,
    replacement_candidate: String,
    delta: Captured,
    slice_contract: Captured,
    findings: Vec<FindingDisposition>,
    reused_validation: Vec<NamedCaptured>,
    affected_validation: Vec<NamedCaptured>,
    delivery_profile_bytes: Vec<u8>,
    max_tokens: usize,
}

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let repository = trusted_repository_root(repository)?;
    let request_bytes = bounded_file::read_regular(
        request_path,
        MAX_REQUEST_BYTES,
        "Slice review delta request",
    )?;
    let request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice review delta request: {error}"))?;
    validate_request(&request)?;
    trusted_ensure_clean(&repository, "building a review delta")?;

    let bound = slice_contract::trusted_bound_slice(&repository)?;
    require_expected_branch(&repository, &bound.base_ref, &bound.slice)?;
    let replacement_candidate = trusted_resolve_commit(&repository, "HEAD")?;
    let manifest_path = resolve_input_path(&repository, &request.prior_manifest_path);
    let prior = verify_chain_head(
        &repository,
        &manifest_path,
        &request.prior_manifest_hash,
        &mut BTreeSet::new(),
        0,
    )?;
    validate_prior(&prior, &bound, &replacement_candidate)?;
    slice_contract::trusted_check_bound_scope(&repository)?;
    let prior_manifest = capture_published(
        &repository,
        &resolve_input_path(&repository, &prior.manifest_path),
        "prior review manifest",
        MAX_INPUT_BYTES,
    )?;
    let prior_packet = capture_published(
        &repository,
        &resolve_input_path(&repository, &prior.packet_path),
        "prior review packet",
        MAX_PACKET_BYTES,
    )?;
    if prior_manifest.hash != prior.manifest_hash || prior_packet.hash != prior.packet_hash {
        return Err("verified prior review artifact identities differ".to_owned());
    }
    let prior_findings = capture_prior_findings(&repository, &request, &prior)?;

    let contract_bytes = bounded_file::read_regular(
        &bound.contract_path,
        MAX_REQUEST_BYTES,
        "bound Slice contract",
    )?;
    let slice_contract = captured(
        bound.contract_path.to_string_lossy().into_owned(),
        contract_bytes,
    )?;
    if slice_contract.hash != bound.contract_id
        || prior.slice_contract_path != slice_contract.path
        || prior.slice_contract_hash != slice_contract.hash
    {
        return Err("prior review and current bound Slice contract differ".to_owned());
    }

    let delta = captured(
        "git-delta.patch".to_owned(),
        capture_delta(&repository, &prior.candidate_commit, &replacement_candidate)?,
    )?;
    if delta.bytes.is_empty() {
        return Err("replacement candidate has no delta from the prior candidate".to_owned());
    }
    let findings = sorted_findings(&request.finding_dispositions)?;
    require_exact_finding_set(&prior_findings, &findings)?;
    let (reused_validation, affected_validation) = capture_validation(
        &repository,
        &prior,
        &request.reused_validation_evidence,
        &request.affected_validation_evidence,
    )?;
    validate_transition(
        &prior,
        &replacement_candidate,
        &delta,
        &findings,
        &reused_validation,
        &affected_validation,
    )?;
    let inputs = Inputs {
        request: captured(request_path.to_string_lossy().into_owned(), request_bytes)?,
        prior_manifest,
        prior_packet,
        prior_findings,
        prior,
        replacement_candidate,
        delta,
        slice_contract,
        findings,
        reused_validation,
        affected_validation,
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: request.max_managed_payload_tokens,
    };

    let plan = build_plan(&inputs);
    let plan_bytes = serde_json::to_vec(&plan).expect("closed review delta plan serializes");
    let review_delta_id = domain_digest(REVIEW_DELTA_ID_DOMAIN, &plan_bytes);
    let packet = render_packet(&review_delta_id, &plan, &inputs)?;
    if packet.len() > MAX_PACKET_BYTES {
        return Err(format!(
            "canonical review delta packet exceeds the {MAX_PACKET_BYTES}-byte safety limit"
        ));
    }
    let managed_payload_tokens = count_tokens(&packet)?;
    require_budget(managed_payload_tokens, inputs.max_tokens)?;
    let packet_hash = digest(&packet);
    let manifest = build_manifest(
        review_delta_id.clone(),
        plan,
        &inputs,
        packet_hash.clone(),
        managed_payload_tokens,
    );
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).expect("closed review delta manifest serializes");
    manifest_bytes.push(b'\n');
    let manifest_hash = digest(&manifest_bytes);
    let suffix = review_delta_id
        .strip_prefix("sha256:")
        .expect("generated review delta ID has a sha256 prefix");
    let output_directory = repository
        .join(".local-exclude/methexis/slice-review-deltas")
        .join(suffix);
    let status = storage::publish(&output_directory, &packet, &manifest_bytes, || {
        final_revalidate(&repository, &request, &inputs)
    })?;

    let result = ResultRecord {
        schema: RESULT_SCHEMA,
        ok: true,
        operation: "build_slice_review_delta",
        status,
        review_delta_id,
        prior_review_id: inputs.prior.review_id.clone(),
        prior_candidate_commit: inputs.prior.candidate_commit.clone(),
        replacement_candidate_commit: inputs.replacement_candidate.clone(),
        packet: ArtifactWithTokens {
            path: relative(&repository, &output_directory.join("packet.md")),
            hash: packet_hash,
            managed_payload_tokens,
        },
        manifest: Artifact {
            path: relative(&repository, &output_directory.join("manifest.json")),
            hash: manifest_hash,
        },
        max_managed_payload_tokens: inputs.max_tokens,
    };
    let mut output = serde_json::to_vec(&result).expect("closed result serializes");
    output.push(b'\n');
    io::Write::write_all(&mut io::stdout().lock(), &output)
        .map_err(|error| format!("cannot write review delta result: {error}"))
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!("expected request schema `{REQUEST_SCHEMA}`"));
    }
    if request.delivery_profile != DELIVERY_PROFILE {
        return Err(format!("expected delivery profile `{DELIVERY_PROFILE}`"));
    }
    if request.tokenizer_profile != TOKENIZER_PROFILE {
        return Err(format!("expected tokenizer profile `{TOKENIZER_PROFILE}`"));
    }
    if request.max_managed_payload_tokens == 0 {
        return Err("managed payload token budget must be positive".to_owned());
    }
    require_hash(&request.prior_manifest_hash, "prior manifest hash")?;
    require_hash(&request.prior_findings_hash, "prior findings hash")?;
    if request.finding_dispositions.is_empty() {
        return Err("at least one finding disposition is required".to_owned());
    }
    if request.affected_validation_evidence.is_empty() {
        return Err(
            "at least one affected validation evidence item is required for a replacement candidate"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_prior(
    prior: &VerifiedReview,
    bound: &slice_contract::BoundSlice,
    replacement_candidate: &str,
) -> Result<(), String> {
    for (value, label) in [
        (prior.base_commit.as_str(), "prior base"),
        (prior.candidate_commit.as_str(), "prior candidate"),
        (prior.trusted_commit.as_str(), "prior trusted commit"),
        (replacement_candidate, "replacement candidate"),
    ] {
        require_commit(value, label)?;
    }
    if prior.base_commit != bound.base {
        return Err("prior review base differs from the bound Slice base".to_owned());
    }
    Ok(())
}

fn verify_chain_head(
    repository: &Path,
    manifest_path: &Path,
    expected_hash: &str,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Result<VerifiedReview, String> {
    verify_chain_head_with(
        repository,
        manifest_path,
        expected_hash,
        seen,
        depth,
        &review_packet::verify_published,
    )
}

fn verify_chain_head_with(
    repository: &Path,
    manifest_path: &Path,
    expected_hash: &str,
    seen: &mut BTreeSet<String>,
    depth: usize,
    verify_original: &dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String>,
) -> Result<VerifiedReview, String> {
    if depth >= 64 {
        return Err("review continuation chain exceeds the 64-hop safety limit".to_owned());
    }
    let manifest_bytes = bounded_file::read_regular(
        manifest_path,
        MAX_INPUT_BYTES,
        "published review-chain manifest",
    )?;
    require_exact_hash(
        expected_hash,
        &manifest_bytes,
        "published review-chain manifest",
    )?;
    if !seen.insert(expected_hash.to_owned()) {
        return Err("review continuation chain contains a cycle".to_owned());
    }
    let schema = serde_json::from_slice::<serde_json::Value>(&manifest_bytes)
        .map_err(|error| format!("invalid published review-chain manifest: {error}"))?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "published review-chain manifest has no schema".to_owned())?
        .to_owned();
    if schema == "yo.slice-review-manifest/v1" {
        return verify_original(repository, manifest_path, expected_hash);
    }
    if schema != MANIFEST_SCHEMA {
        return Err(format!(
            "unsupported review-chain manifest schema `{schema}`"
        ));
    }

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid published review delta manifest: {error}"))?;
    require_hash(&manifest.review_delta_id, "published ReviewDeltaId")?;
    if manifest.plan.schema != PLAN_SCHEMA
        || manifest.plan.delivery_profile != delivery_profile()
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
        || manifest.plan.tokenizer_compiler != TOKENIZER_COMPILER
        || manifest.packet.path != "packet.md"
        || manifest.packet.max_managed_payload_tokens != manifest.plan.max_managed_payload_tokens
    {
        return Err("published review delta manifest uses an unsupported contract".to_owned());
    }
    let expected_path = repository
        .join(".local-exclude/methexis/slice-review-deltas")
        .join(
            manifest
                .review_delta_id
                .strip_prefix("sha256:")
                .ok_or_else(|| "published ReviewDeltaId is not a SHA-256 identity".to_owned())?,
        )
        .join("manifest.json");
    if std::fs::canonicalize(manifest_path)
        .map_err(|error| format!("cannot resolve published delta manifest: {error}"))?
        != std::fs::canonicalize(&expected_path)
            .map_err(|error| format!("cannot resolve published ReviewDeltaId directory: {error}"))?
    {
        return Err("manifest is not the exact published ReviewDeltaId artifact".to_owned());
    }

    let prior_path = resolve_input_path(repository, &manifest.inputs.prior_manifest.path);
    let prior = verify_chain_head_with(
        repository,
        &prior_path,
        &manifest.inputs.prior_manifest.hash,
        seen,
        depth + 1,
        verify_original,
    )?;
    let prior_manifest = capture_published(
        repository,
        &prior_path,
        "prior review-chain manifest",
        MAX_INPUT_BYTES,
    )?;
    let prior_packet = capture_published(
        repository,
        &resolve_input_path(repository, &manifest.inputs.prior_packet.path),
        "prior review-chain packet",
        MAX_PACKET_BYTES,
    )?;
    if artifact(&prior_manifest) != manifest.inputs.prior_manifest
        || artifact(&prior_packet) != manifest.inputs.prior_packet
        || prior.packet_hash != prior_packet.hash
    {
        return Err("published review delta prior artifact identities differ".to_owned());
    }
    let prior_findings = capture_file(
        &resolve_input_path(repository, &manifest.inputs.prior_findings.path),
        "prior review findings",
    )?;
    if artifact(&prior_findings) != manifest.inputs.prior_findings {
        return Err("published review delta prior findings identity differs".to_owned());
    }
    validate_findings_artifact(&prior_findings, &prior)?;
    require_exact_finding_set(&prior_findings, &manifest.plan.finding_dispositions)?;

    let slice_contract = capture_file(
        &resolve_input_path(repository, &manifest.inputs.slice_contract.path),
        "Slice contract",
    )?;
    if artifact(&slice_contract) != manifest.inputs.slice_contract
        || slice_contract.path != prior.slice_contract_path
        || slice_contract.hash != prior.slice_contract_hash
    {
        return Err("published review delta Slice contract identity differs".to_owned());
    }
    let reused_validation = capture_named_artifacts(
        repository,
        &manifest.inputs.reused_validation_evidence,
        "reused validation evidence",
    )?;
    let affected_validation = capture_named_artifacts(
        repository,
        &manifest.inputs.affected_validation_evidence,
        "affected validation evidence",
    )?;
    let mut aggregate = 0usize;
    for evidence in reused_validation.iter().chain(&affected_validation) {
        add_evidence_size(&mut aggregate, evidence)?;
    }
    let replacement_candidate = manifest.plan.replacement_candidate_commit.clone();
    let delta = captured(
        "git-delta.patch".to_owned(),
        capture_delta(repository, &prior.candidate_commit, &replacement_candidate)?,
    )?;
    validate_transition(
        &prior,
        &replacement_candidate,
        &delta,
        &manifest.plan.finding_dispositions,
        &reused_validation,
        &affected_validation,
    )?;
    let inputs = Inputs {
        request: captured(
            "verified-chain-request".to_owned(),
            b"verified chain".to_vec(),
        )?,
        prior_manifest,
        prior_packet,
        prior_findings,
        prior,
        replacement_candidate,
        delta,
        slice_contract,
        findings: manifest.plan.finding_dispositions.clone(),
        reused_validation,
        affected_validation,
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: manifest.plan.max_managed_payload_tokens,
    };
    let plan = build_plan(&inputs);
    if plan != manifest.plan {
        return Err("published review delta plan does not reproduce from its inputs".to_owned());
    }
    let review_delta_id = domain_digest(
        REVIEW_DELTA_ID_DOMAIN,
        &serde_json::to_vec(&plan).expect("review delta plan serializes"),
    );
    if review_delta_id != manifest.review_delta_id {
        return Err("published ReviewDeltaId does not match its canonical plan".to_owned());
    }
    let packet_path = expected_path
        .parent()
        .expect("published delta manifest has a parent")
        .join("packet.md");
    let packet = capture_packet(&packet_path, "published review delta packet")?;
    let reproduced_packet = render_packet(&review_delta_id, &plan, &inputs)?;
    if packet.bytes != reproduced_packet || packet.hash != manifest.packet.hash {
        return Err("published review delta packet does not reproduce exactly".to_owned());
    }
    let tokens = count_tokens(&packet.bytes)?;
    if tokens != manifest.packet.managed_payload_tokens
        || tokens > manifest.packet.max_managed_payload_tokens
    {
        return Err("published review delta token record does not match exact bytes".to_owned());
    }
    let reproduced_manifest = build_manifest(
        review_delta_id.clone(),
        plan,
        &inputs,
        packet.hash.clone(),
        tokens,
    );
    let mut reproduced_manifest_bytes =
        serde_json::to_vec_pretty(&reproduced_manifest).expect("review delta manifest serializes");
    reproduced_manifest_bytes.push(b'\n');
    if reproduced_manifest_bytes != manifest_bytes {
        return Err("published review delta manifest bytes are not canonical".to_owned());
    }

    Ok(VerifiedReview {
        review_id: review_delta_id,
        manifest_path: relative(repository, &expected_path),
        manifest_hash: digest(&manifest_bytes),
        packet_path: relative(repository, &packet_path),
        packet_hash: packet.hash,
        base_commit: inputs.prior.base_commit,
        candidate_commit: inputs.replacement_candidate,
        trusted_commit: inputs.prior.trusted_commit,
        slice_contract_path: inputs.slice_contract.path,
        slice_contract_hash: inputs.slice_contract.hash,
        validation_evidence: inputs
            .reused_validation
            .into_iter()
            .chain(inputs.affected_validation)
            .map(|evidence| review_packet::VerifiedEvidence {
                name: evidence.name,
                path: evidence.artifact.path,
                hash: evidence.artifact.hash,
            })
            .collect(),
        review_lenses: inputs.prior.review_lenses,
        review_questions: inputs.prior.review_questions,
    })
}

fn validate_transition(
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
        if let Some(previous) = prior_by_name.get(evidence.name.as_str())
            && previous.path == evidence.artifact.path
            && previous.hash == evidence.artifact.hash
        {
            return Err(format!(
                "affected validation evidence `{}` is unchanged from the prior candidate",
                evidence.name
            ));
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

fn validate_findings_artifact(captured: &Captured, prior: &VerifiedReview) -> Result<(), String> {
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

fn capture_named_artifacts(
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

fn capture_prior_findings(
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

fn capture_validation(
    repository: &Path,
    prior: &VerifiedReview,
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
        let item = NamedCaptured {
            name: request.name.clone(),
            artifact: capture_file(
                &resolve_input_path(repository, &request.path),
                "affected validation evidence",
            )?,
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

fn add_evidence_size(total: &mut usize, evidence: &NamedCaptured) -> Result<(), String> {
    add_evidence_bytes(total, evidence.artifact.bytes.len())
}

fn add_evidence_bytes(total: &mut usize, bytes: usize) -> Result<(), String> {
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

fn sorted_findings(values: &[FindingDisposition]) -> Result<Vec<FindingDisposition>, String> {
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

fn require_exact_finding_set(
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

fn build_plan(inputs: &Inputs) -> ReviewDeltaPlan {
    ReviewDeltaPlan {
        schema: PLAN_SCHEMA.to_owned(),
        prior_review_id: inputs.prior.review_id.clone(),
        prior_manifest_hash: inputs.prior_manifest.hash.clone(),
        prior_packet_hash: inputs.prior_packet.hash.clone(),
        prior_findings: artifact(&inputs.prior_findings),
        prior_candidate_commit: inputs.prior.candidate_commit.clone(),
        replacement_candidate_commit: inputs.replacement_candidate.clone(),
        delta_hash: inputs.delta.hash.clone(),
        trusted_commit: inputs.prior.trusted_commit.clone(),
        slice_contract: artifact(&inputs.slice_contract),
        finding_dispositions: inputs.findings.clone(),
        reused_validation_evidence: inputs
            .reused_validation
            .iter()
            .map(named_semantic_input)
            .collect(),
        affected_validation_evidence: inputs
            .affected_validation
            .iter()
            .map(named_semantic_input)
            .collect(),
        review_lenses: inputs.prior.review_lenses.clone(),
        review_questions: inputs.prior.review_questions.clone(),
        delivery_profile: delivery_profile(),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        tokenizer_compiler: TOKENIZER_COMPILER.to_owned(),
        max_managed_payload_tokens: inputs.max_tokens,
    }
}

fn render_packet(
    review_delta_id: &str,
    plan: &ReviewDeltaPlan,
    inputs: &Inputs,
) -> Result<Vec<u8>, String> {
    let mut packet = PREAMBLE.as_bytes().to_vec();
    let plan_bytes = serde_json::to_vec_pretty(plan).expect("closed delta plan serializes");
    append_section(
        &mut packet,
        "review_delta_plan",
        review_delta_id,
        "",
        &plan_bytes,
    )?;
    let findings =
        serde_json::to_vec_pretty(&inputs.findings).expect("finding dispositions serialize");
    append_section(
        &mut packet,
        "prior_findings",
        "exact-prior-finding-set",
        &inputs.prior_findings.path,
        &inputs.prior_findings.bytes,
    )?;
    append_section(
        &mut packet,
        "finding_dispositions",
        "exact-finding-dispositions",
        "",
        &findings,
    )?;
    let reused = serde_json::to_vec_pretty(
        &inputs
            .reused_validation
            .iter()
            .map(named_artifact)
            .collect::<Vec<_>>(),
    )
    .expect("reused evidence records serialize");
    append_section(
        &mut packet,
        "reused_validation_evidence",
        "unchanged-green-evidence",
        "",
        &reused,
    )?;
    for evidence in &inputs.affected_validation {
        append_section(
            &mut packet,
            "affected_validation_evidence",
            &evidence.name,
            &evidence.artifact.path,
            &evidence.artifact.bytes,
        )?;
    }
    append_section(
        &mut packet,
        "git_delta",
        "prior-to-replacement-candidate",
        &inputs.delta.path,
        &inputs.delta.bytes,
    )?;
    packet.extend_from_slice(PAYLOAD_SUFFIX.as_bytes());
    Ok(packet)
}

#[derive(Serialize)]
struct SectionMetadata<'a> {
    kind: &'a str,
    name: &'a str,
    path: &'a str,
    hash: String,
    bytes: usize,
}

fn append_section(
    output: &mut Vec<u8>,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    std::str::from_utf8(bytes)
        .map_err(|_| format!("review delta section `{name}` is not UTF-8 model-visible text"))?;
    let metadata = serde_json::to_vec(&SectionMetadata {
        kind,
        name,
        path,
        hash: digest(bytes),
        bytes: bytes.len(),
    })
    .expect("section metadata serializes");
    output.extend_from_slice(SECTION_PREFIX.as_bytes());
    output.extend_from_slice(&metadata);
    output.extend_from_slice(METADATA_SUFFIX.as_bytes());
    output.extend_from_slice(bytes);
    output.extend_from_slice(SECTION_SUFFIX.as_bytes());
    Ok(())
}

fn build_manifest(
    review_delta_id: String,
    plan: ReviewDeltaPlan,
    inputs: &Inputs,
    packet_hash: String,
    managed_payload_tokens: usize,
) -> Manifest {
    Manifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        review_delta_id,
        plan,
        inputs: ManifestInputs {
            prior_manifest: artifact(&inputs.prior_manifest),
            prior_packet: artifact(&inputs.prior_packet),
            prior_findings: artifact(&inputs.prior_findings),
            slice_contract: artifact(&inputs.slice_contract),
            reused_validation_evidence: inputs
                .reused_validation
                .iter()
                .map(named_artifact)
                .collect(),
            affected_validation_evidence: inputs
                .affected_validation
                .iter()
                .map(named_artifact)
                .collect(),
            delta: artifact(&inputs.delta),
        },
        packet: PacketRecord {
            path: "packet.md".to_owned(),
            hash: packet_hash,
            managed_payload_tokens,
            max_managed_payload_tokens: inputs.max_tokens,
        },
    }
}

fn final_revalidate(repository: &Path, request: &Request, inputs: &Inputs) -> Result<(), String> {
    trusted_ensure_clean(repository, "returning a review delta")?;
    let bound = slice_contract::trusted_bound_slice(repository)?;
    require_expected_branch(repository, &bound.base_ref, &bound.slice)?;
    slice_contract::trusted_check_bound_scope(repository)?;
    if trusted_resolve_commit(repository, "HEAD")? != inputs.replacement_candidate {
        return Err("replacement candidate changed during delta construction".to_owned());
    }
    let verified = verify_chain_head(
        repository,
        &resolve_input_path(repository, &inputs.prior_manifest.path),
        &inputs.prior_manifest.hash,
        &mut BTreeSet::new(),
        0,
    )?;
    if verified.review_id != inputs.prior.review_id
        || verified.packet_hash != inputs.prior.packet_hash
        || verified.candidate_commit != inputs.prior.candidate_commit
        || verified.trusted_commit != inputs.prior.trusted_commit
    {
        return Err("prior published review identity changed during delta construction".to_owned());
    }
    if capture_delta(
        repository,
        &inputs.prior.candidate_commit,
        &inputs.replacement_candidate,
    )? != inputs.delta.bytes
    {
        return Err("prior-to-replacement delta changed during construction".to_owned());
    }
    require_current_file(
        &resolve_input_path(repository, &inputs.request.path),
        &inputs.request,
        "review delta request",
    )?;
    require_current_file(
        &resolve_input_path(repository, &inputs.prior_manifest.path),
        &inputs.prior_manifest,
        "prior review manifest",
    )?;
    require_current_packet(
        &resolve_input_path(repository, &inputs.prior_packet.path),
        &inputs.prior_packet,
        "prior review packet",
    )?;
    require_current_file(
        Path::new(&inputs.prior_findings.path),
        &inputs.prior_findings,
        "prior review findings",
    )?;
    require_exact_finding_set(&inputs.prior_findings, &inputs.findings)?;
    require_current_file(
        Path::new(&inputs.slice_contract.path),
        &inputs.slice_contract,
        "Slice contract",
    )?;
    if bound.base != inputs.prior.base_commit
        || bound.contract_id != inputs.slice_contract.hash
        || bound.contract_path != Path::new(&inputs.slice_contract.path)
    {
        return Err("bound Slice contract identity changed during delta construction".to_owned());
    }
    let (reused, affected) = capture_validation(
        repository,
        &inputs.prior,
        &request.reused_validation_evidence,
        &request.affected_validation_evidence,
    )?;
    require_named_captures(&reused, &inputs.reused_validation)?;
    require_named_captures(&affected, &inputs.affected_validation)?;
    if delivery_profile_bytes() != inputs.delivery_profile_bytes {
        return Err("delivery profile bytes changed during delta construction".to_owned());
    }
    Ok(())
}

fn capture_delta(repository: &Path, prior: &str, replacement: &str) -> Result<Vec<u8>, String> {
    if !git::trusted_succeeds_in(
        repository,
        &["merge-base", "--is-ancestor", prior, replacement],
    )? {
        return Err("prior candidate is not an ancestor of the replacement candidate".to_owned());
    }
    git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            prior,
            replacement,
            "--",
        ],
    )
}

fn capture_file(path: &Path, label: &str) -> Result<Captured, String> {
    let bytes = bounded_file::read_regular(path, MAX_INPUT_BYTES, label)?;
    captured(path.to_string_lossy().into_owned(), bytes)
}

fn capture_packet(path: &Path, label: &str) -> Result<Captured, String> {
    let bytes = bounded_file::read_regular(path, MAX_PACKET_BYTES, label)?;
    std::str::from_utf8(&bytes).map_err(|_| {
        format!(
            "review delta input `{}` is not UTF-8 model-visible text",
            path.display()
        )
    })?;
    Ok(Captured {
        path: path.to_string_lossy().into_owned(),
        hash: digest(&bytes),
        bytes,
    })
}

fn capture_published(
    repository: &Path,
    path: &Path,
    label: &str,
    maximum: usize,
) -> Result<Captured, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve {label} {}: {error}", path.display()))?;
    let bytes = bounded_file::read_regular(&canonical, maximum, label)?;
    std::str::from_utf8(&bytes).map_err(|_| format!("{label} is not UTF-8 model-visible text"))?;
    Ok(Captured {
        path: relative(repository, &canonical),
        hash: digest(&bytes),
        bytes,
    })
}

fn require_current_file(path: &Path, expected: &Captured, label: &str) -> Result<(), String> {
    let actual = capture_file(path, label)?;
    if actual.hash == expected.hash && actual.bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during review delta construction"))
    }
}

fn require_current_packet(path: &Path, expected: &Captured, label: &str) -> Result<(), String> {
    let actual = capture_packet(path, label)?;
    if actual.hash == expected.hash && actual.bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during review delta construction"))
    }
}

fn require_named_captures(
    actual: &[NamedCaptured],
    expected: &[NamedCaptured],
) -> Result<(), String> {
    if actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(left, right)| {
            left.name == right.name
                && left.artifact.path == right.artifact.path
                && left.artifact.hash == right.artifact.hash
                && left.artifact.bytes == right.artifact.bytes
        })
    {
        Ok(())
    } else {
        Err("validation evidence changed during review delta construction".to_owned())
    }
}

fn delivery_profile() -> DeliveryProfile {
    DeliveryProfile {
        id: DELIVERY_PROFILE.to_owned(),
        preamble: PREAMBLE.to_owned(),
        section_prefix: SECTION_PREFIX.to_owned(),
        metadata_suffix: METADATA_SUFFIX.to_owned(),
        section_suffix: SECTION_SUFFIX.to_owned(),
        payload_suffix: PAYLOAD_SUFFIX.to_owned(),
    }
}

fn delivery_profile_bytes() -> Vec<u8> {
    serde_json::to_vec(&delivery_profile()).expect("closed delivery profile serializes")
}

fn count_tokens(bytes: &[u8]) -> Result<usize, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "canonical review delta packet is not UTF-8".to_owned())?;
    Ok(tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(text)
        .len())
}

fn require_budget(actual: usize, maximum: usize) -> Result<(), String> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(format!(
            "managed delta payload requires {actual} tokens but the budget is {maximum}; no content was truncated"
        ))
    }
}

fn captured(path: String, bytes: Vec<u8>) -> Result<Captured, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "review delta input `{path}` exceeds the {MAX_INPUT_BYTES}-byte limit"
        ));
    }
    std::str::from_utf8(&bytes)
        .map_err(|_| format!("review delta input `{path}` is not UTF-8 model-visible text"))?;
    Ok(Captured {
        path,
        hash: digest(&bytes),
        bytes,
    })
}

fn named_artifact(input: &NamedCaptured) -> NamedArtifact {
    NamedArtifact {
        name: input.name.clone(),
        artifact: artifact(&input.artifact),
    }
}

fn named_semantic_input(input: &NamedCaptured) -> NamedSemanticInput {
    NamedSemanticInput {
        name: input.name.clone(),
        path: input.artifact.path.clone(),
        hash: input.artifact.hash.clone(),
    }
}

fn require_hash(value: &str, label: &str) -> Result<(), String> {
    let valid = value.strip_prefix("sha256:").is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    });
    if valid {
        Ok(())
    } else {
        Err(format!("{label} must be a canonical SHA-256 identity"))
    }
}

fn require_exact_hash(expected: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    require_hash(expected, label)?;
    let actual = digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}

fn require_expected_branch(repository: &Path, base_ref: &str, slice: &str) -> Result<(), String> {
    let expected = if base_ref == "refs/heads/develop" {
        format!("refs/heads/slice/direct/{slice}")
    } else {
        let wave = base_ref
            .strip_prefix("refs/heads/wave/")
            .filter(|wave| !wave.is_empty() && !wave.contains('/'))
            .ok_or_else(|| format!("unsupported Slice integration ref `{base_ref}`"))?;
        format!("refs/heads/slice/{wave}/{slice}")
    };
    let actual = git::trusted_output_in(repository, &["symbolic-ref", "--quiet", "HEAD"])?;
    if actual.trim() == expected {
        Ok(())
    } else {
        Err(format!(
            "trusted Git branch does not match bound Slice; expected {expected}"
        ))
    }
}

fn trusted_resolve_commit(repository: &Path, reference: &str) -> Result<String, String> {
    let value = git::trusted_output_in(
        repository,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )?;
    let value = value.trim().to_owned();
    require_commit(&value, "resolved commit")?;
    Ok(value)
}

fn trusted_repository_root(directory: &Path) -> Result<PathBuf, String> {
    let root = git::trusted_output_in(directory, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim();
    if root.is_empty() {
        return Err("trusted Git returned an empty repository root".to_owned());
    }
    Ok(PathBuf::from(root))
}

fn trusted_ensure_clean(repository: &Path, operation: &str) -> Result<(), String> {
    let status = git::trusted_output_bytes_in(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "candidate worktree must be clean before {operation}"
        ))
    }
}
