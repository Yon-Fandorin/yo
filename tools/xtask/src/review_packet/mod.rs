mod model;
pub(crate) mod storage;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use serde::Serialize;

use self::model::{
    Artifact, ArtifactWithTokens, CheckpointIdentity, ContextManifest, ContextResult,
    DELIVERY_PROFILE, DeliveryProfile, EvidenceRequest, MANIFEST_SCHEMA, Manifest, ManifestInputs,
    NamedArtifact, NamedSemanticInput, PLAN_SCHEMA, PacketRecord, REQUEST_SCHEMA, RESULT_SCHEMA,
    Request, ResultRecord, ReviewPlan, SemanticInput, TOKENIZER_COMPILER, TOKENIZER_PROFILE,
};
use crate::{
    bounded_file, git,
    review_protocol::{
        Captured, NamedCaptured, artifact, digest, domain_digest, relative, require_commit,
        resolve_input_path, sorted_unique,
    },
    slice_contract,
};

const REVIEW_ID_DOMAIN: &[u8] = b"yo.slice-review/v1";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
const PREAMBLE: &str = "# yo Slice Review Packet\n\nThis packet is the complete caller-controlled model-visible review payload. Review only the immutable candidate described here against the included authority, contract, evidence, and questions.\n";
const SECTION_PREFIX: &str = "\n<<<YO-REVIEW-SECTION ";
const METADATA_SUFFIX: &str = ">>>\n";
const SECTION_SUFFIX: &str = "\n<<<YO-REVIEW-SECTION-END>>>\n";
const PAYLOAD_SUFFIX: &str = "\n<<<YO-REVIEW-PAYLOAD-END>>>\n";

struct ContextCapture {
    result: ContextResult,
    request: Captured,
    context: Captured,
    manifest: Captured,
    active_checkpoint: CheckpointIdentity,
    included_ids: Vec<String>,
}

struct Inputs {
    base_commit: String,
    candidate_commit: String,
    diff: Captured,
    context: ContextCapture,
    authorities: Vec<Captured>,
    slice_contract: Captured,
    validation: Vec<NamedCaptured>,
    lenses: Vec<String>,
    questions: Vec<String>,
    required_knowledge_ids: Vec<String>,
    delivery_profile_bytes: Vec<u8>,
    max_tokens: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedEvidence {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) hash: String,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedReview {
    pub(crate) review_id: String,
    pub(crate) manifest_path: String,
    pub(crate) manifest_hash: String,
    pub(crate) packet_path: String,
    pub(crate) packet_hash: String,
    pub(crate) base_commit: String,
    pub(crate) candidate_commit: String,
    pub(crate) trusted_commit: String,
    pub(crate) slice_contract_path: String,
    pub(crate) slice_contract_hash: String,
    pub(crate) validation_evidence: Vec<VerifiedEvidence>,
    pub(crate) review_lenses: Vec<String>,
    pub(crate) review_questions: Vec<String>,
}

pub(crate) fn verify_published(
    repository: &Path,
    manifest_path: &Path,
    expected_manifest_hash: &str,
) -> Result<VerifiedReview, String> {
    let repository = trusted_repository_root(repository)?;
    let manifest_bytes = bounded_file::read_regular(
        manifest_path,
        MAX_INPUT_BYTES,
        "published Slice review manifest",
    )?;
    require_hash(
        expected_manifest_hash,
        &manifest_bytes,
        "published Slice review manifest",
    )?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid published Slice review manifest: {error}"))?;
    require_supported_manifest(&manifest)?;
    let plan_bytes = serde_json::to_vec(&manifest.plan).expect("review plan serializes");
    let reproduced_id = domain_digest(REVIEW_ID_DOMAIN, &plan_bytes);
    if reproduced_id != manifest.review_id {
        return Err("published ReviewId does not match its canonical plan".to_owned());
    }
    let suffix = manifest
        .review_id
        .strip_prefix("sha256:")
        .expect("reproduced ReviewId has a prefix");
    let expected_path = repository
        .join(".local-exclude/methexis/slice-reviews")
        .join(suffix)
        .join("manifest.json");
    if std::fs::canonicalize(manifest_path)
        .map_err(|error| format!("cannot resolve published review manifest: {error}"))?
        != std::fs::canonicalize(&expected_path)
            .map_err(|error| format!("cannot resolve published ReviewId directory: {error}"))?
    {
        return Err("manifest is not the exact published ReviewId artifact".to_owned());
    }

    if trusted_resolve_commit(&repository, "refs/heads/develop")? != manifest.plan.trusted_commit {
        return Err("trusted integration changed since the published review".to_owned());
    }
    let context_request_path =
        resolve_input_path(&repository, &manifest.inputs.context_request.path);
    let context = capture_context(&repository, &context_request_path)?;
    let authorities = capture_authorities(
        &repository,
        &manifest.plan.candidate_commit,
        &manifest
            .plan
            .repository_authorities
            .iter()
            .map(|input| input.path.clone())
            .collect::<Vec<_>>(),
    )?;
    let validation_requests = manifest
        .inputs
        .validation_evidence
        .iter()
        .map(|input| EvidenceRequest {
            name: input.name.clone(),
            path: input.artifact.path.clone(),
        })
        .collect::<Vec<_>>();
    let validation = capture_validation(&repository, &validation_requests)?;
    let contract_path = resolve_input_path(&repository, &manifest.inputs.slice_contract.path);
    let contract_bytes =
        bounded_file::read_regular(&contract_path, MAX_REQUEST_BYTES, "Slice review contract")?;
    let slice_contract = Captured {
        path: contract_path.to_string_lossy().into_owned(),
        hash: digest(&contract_bytes),
        bytes: contract_bytes,
    };
    let inputs = Inputs {
        base_commit: manifest.plan.base_commit.clone(),
        candidate_commit: manifest.plan.candidate_commit.clone(),
        diff: captured(
            "git-diff.patch".to_owned(),
            capture_diff(
                &repository,
                &manifest.plan.base_commit,
                &manifest.plan.candidate_commit,
            )?,
        )?,
        context,
        authorities,
        slice_contract,
        validation,
        lenses: manifest.plan.review_lenses.clone(),
        questions: manifest.plan.review_questions.clone(),
        required_knowledge_ids: manifest.plan.required_knowledge_ids.clone(),
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: manifest.plan.max_managed_payload_tokens,
    };
    let packet_path = expected_path
        .parent()
        .expect("published manifest path has a parent")
        .join("packet.md");
    let packet_bytes =
        bounded_file::read_regular(&packet_path, MAX_PACKET_BYTES, "published review packet")?;
    verify_canonical_artifacts(&manifest, &manifest_bytes, &packet_bytes, &inputs)?;

    Ok(VerifiedReview {
        review_id: manifest.review_id,
        manifest_path: relative(&repository, &expected_path),
        manifest_hash: digest(&manifest_bytes),
        packet_path: relative(&repository, &packet_path),
        packet_hash: manifest.packet.hash,
        base_commit: inputs.base_commit,
        candidate_commit: inputs.candidate_commit,
        trusted_commit: inputs.context.result.trusted_commit,
        slice_contract_path: inputs.slice_contract.path,
        slice_contract_hash: inputs.slice_contract.hash,
        validation_evidence: inputs
            .validation
            .into_iter()
            .map(|input| VerifiedEvidence {
                name: input.name,
                path: input.artifact.path,
                hash: input.artifact.hash,
            })
            .collect(),
        review_lenses: inputs.lenses,
        review_questions: inputs.questions,
    })
}

fn verify_canonical_artifacts(
    manifest: &Manifest,
    manifest_bytes: &[u8],
    packet_bytes: &[u8],
    inputs: &Inputs,
) -> Result<(), String> {
    require_supported_manifest(manifest)?;
    let reproduced_plan = build_plan(inputs);
    if reproduced_plan != manifest.plan {
        return Err("published review plan does not match its complete captured inputs".to_owned());
    }
    let plan_bytes = serde_json::to_vec(&reproduced_plan).expect("review plan serializes");
    if domain_digest(REVIEW_ID_DOMAIN, &plan_bytes) != manifest.review_id {
        return Err("published ReviewId does not match its canonical plan".to_owned());
    }
    let reproduced_packet = render_packet(&manifest.review_id, &reproduced_plan, inputs)?;
    if packet_bytes != reproduced_packet {
        return Err(
            "published review packet does not reproduce from its complete inputs".to_owned(),
        );
    }
    require_hash(
        &manifest.packet.hash,
        packet_bytes,
        "published review packet",
    )?;
    let tokens = count_tokens(packet_bytes)?;
    if tokens != manifest.packet.managed_payload_tokens
        || tokens > manifest.packet.max_managed_payload_tokens
    {
        return Err(
            "published review packet token record does not match its exact bytes".to_owned(),
        );
    }
    let reproduced_manifest = build_manifest(
        manifest.review_id.clone(),
        reproduced_plan,
        inputs,
        manifest.packet.hash.clone(),
        tokens,
    );
    let mut reproduced_manifest_bytes = serde_json::to_vec_pretty(&reproduced_manifest)
        .expect("published review manifest serializes");
    reproduced_manifest_bytes.push(b'\n');
    if reproduced_manifest_bytes != manifest_bytes {
        return Err("published review manifest bytes are not canonical".to_owned());
    }
    Ok(())
}

fn require_supported_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema != MANIFEST_SCHEMA
        || manifest.plan.schema != PLAN_SCHEMA
        || manifest.plan.delivery_profile != delivery_profile()
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
        || manifest.plan.tokenizer_compiler != TOKENIZER_COMPILER
        || manifest.packet.path != "packet.md"
        || manifest.packet.max_managed_payload_tokens != manifest.plan.max_managed_payload_tokens
    {
        return Err("published Slice review manifest uses an unsupported contract".to_owned());
    }
    Ok(())
}

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let repository = trusted_repository_root(repository)?;
    let request_bytes = bounded_file::read_regular(
        request_path,
        MAX_REQUEST_BYTES,
        "Slice review packet request",
    )?;
    let request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice review packet request: {error}"))?;
    validate_request(&request)?;
    trusted_ensure_clean(
        &repository,
        "candidate worktree",
        "building a review packet",
    )?;

    let candidate_commit = trusted_resolve_commit(&repository, "HEAD")?;
    let contract_path = resolve_input_path(&repository, &request.slice_contract_path);
    let contract_bytes =
        bounded_file::read_regular(&contract_path, MAX_REQUEST_BYTES, "Slice review contract")?;
    let bound = slice_contract::trusted_bound_slice(&repository)?;
    let canonical_contract = std::fs::canonicalize(&contract_path)
        .map_err(|error| format!("cannot resolve Slice contract: {error}"))?;
    if canonical_contract != bound.contract_path || digest(&contract_bytes) != bound.contract_id {
        return Err("request does not identify the exact bound Slice contract".to_owned());
    }
    let branch = trusted_git_text(&repository, &["symbolic-ref", "--quiet", "HEAD"])?;
    let expected_branch = expected_slice_ref(&bound.base_ref, &bound.slice)?;
    if branch.trim() != expected_branch {
        return Err(format!(
            "trusted Git branch does not match bound Slice; expected {expected_branch}"
        ));
    }
    require_commit(&bound.base, "Slice base")?;
    let base_commit = trusted_resolve_commit(&repository, &bound.base)?;
    if !trusted_git_succeeds(
        &repository,
        &[
            "merge-base",
            "--is-ancestor",
            &base_commit,
            &candidate_commit,
        ],
    )? {
        return Err("Slice base is not an ancestor of the candidate commit".to_owned());
    }

    let context_request_path = resolve_input_path(&repository, &request.context_request_path);
    let context = capture_context(&repository, &context_request_path)?;
    let required_knowledge_ids =
        sorted_unique(&request.required_knowledge_ids, "required KnowledgeId")?;
    let included = context.included_ids.iter().collect::<BTreeSet<_>>();
    if required_knowledge_ids
        .iter()
        .any(|knowledge_id| !included.contains(knowledge_id))
    {
        return Err("ContextBuild does not include every required KnowledgeId".to_owned());
    }
    let diff_bytes = capture_diff(&repository, &base_commit, &candidate_commit)?;
    let diff = captured("git-diff.patch".to_owned(), diff_bytes)?;
    let authorities = capture_authorities(
        &repository,
        &candidate_commit,
        &request.repository_authority_paths,
    )?;
    let validation = capture_validation(&repository, &request.validation_evidence)?;
    let slice_contract = Captured {
        path: contract_path.to_string_lossy().into_owned(),
        hash: digest(&contract_bytes),
        bytes: contract_bytes,
    };
    let inputs = Inputs {
        base_commit,
        candidate_commit,
        diff,
        context,
        authorities,
        slice_contract,
        validation,
        lenses: sorted_unique(&request.review_lenses, "review lens")?,
        questions: request.review_questions.clone(),
        required_knowledge_ids,
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: request.max_managed_payload_tokens,
    };

    let plan = build_plan(&inputs);
    let plan_bytes = serde_json::to_vec(&plan).expect("closed review plan serializes");
    let review_id = domain_digest(REVIEW_ID_DOMAIN, &plan_bytes);
    let packet = render_packet(&review_id, &plan, &inputs)?;
    if packet.len() > MAX_PACKET_BYTES {
        return Err(format!(
            "canonical review packet exceeds the {MAX_PACKET_BYTES}-byte safety limit"
        ));
    }
    let managed_payload_tokens = count_tokens(&packet)?;
    require_budget(managed_payload_tokens, inputs.max_tokens)?;
    let packet_hash = digest(&packet);
    let manifest = build_manifest(
        review_id.clone(),
        plan,
        &inputs,
        packet_hash.clone(),
        managed_payload_tokens,
    );
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).expect("closed review manifest serializes");
    manifest_bytes.push(b'\n');
    let manifest_hash = digest(&manifest_bytes);
    let suffix = review_id
        .strip_prefix("sha256:")
        .expect("generated ReviewId has a sha256 prefix");
    let output_directory = repository
        .join(".local-exclude/methexis/slice-reviews")
        .join(suffix);
    let status = storage::publish(&output_directory, &packet, &manifest_bytes, || {
        final_revalidate(&repository, &request, &inputs)
    })?;

    let result = ResultRecord {
        schema: RESULT_SCHEMA,
        ok: true,
        operation: "build_slice_review_packet",
        status,
        review_id,
        trusted_commit: inputs.context.result.trusted_commit.clone(),
        candidate_commit: inputs.candidate_commit.clone(),
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
        .map_err(|error| format!("cannot write review packet result: {error}"))
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
    if request.repository_authority_paths.is_empty()
        || request.validation_evidence.is_empty()
        || request.required_knowledge_ids.is_empty()
        || request.review_lenses.is_empty()
        || request.review_questions.is_empty()
    {
        return Err(
            "authority paths, validation evidence, review lenses, and questions must be non-empty"
                .to_owned(),
        );
    }
    if request
        .review_questions
        .iter()
        .any(|question| question.trim().is_empty())
    {
        return Err("review questions must not be blank".to_owned());
    }
    Ok(())
}

fn capture_context(repository: &Path, request_path: &Path) -> Result<ContextCapture, String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        MAX_REQUEST_BYTES,
        "Methexis ContextBuild request",
    )?;
    let result = resolve_context(request_path)?;
    if result.schema != "methexis.context-result/v1alpha1"
        || !result.ok
        || result.operation != "resolve_context"
        || result.authority != "trusted_integration"
    {
        return Err("Methexis returned a non-success ContextBuild result".to_owned());
    }
    let context_path = repository.join(&result.context.path);
    let manifest_path = repository.join(&result.manifest.path);
    let context_bytes = bounded_file::read_regular(
        &context_path,
        MAX_INPUT_BYTES,
        "Methexis ContextBuild context",
    )?;
    let manifest_bytes = bounded_file::read_regular(
        &manifest_path,
        MAX_INPUT_BYTES,
        "Methexis ContextBuild manifest",
    )?;
    require_hash(&result.context.hash, &context_bytes, "ContextBuild context")?;
    require_hash(
        &result.manifest.hash,
        &manifest_bytes,
        "ContextBuild manifest",
    )?;
    let manifest: ContextManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid ContextBuild manifest: {error}"))?;
    if manifest.schema != "methexis.context-manifest/v1alpha1"
        || manifest.build_id != result.build_id
        || manifest.context.hash != result.context.hash
        || manifest.context.path != "context.md"
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
    {
        return Err("ContextBuild result and manifest identities differ".to_owned());
    }
    Ok(ContextCapture {
        result,
        request: captured(request_path.to_string_lossy().into_owned(), request_bytes)?,
        context: captured(context_path.to_string_lossy().into_owned(), context_bytes)?,
        manifest: captured(manifest_path.to_string_lossy().into_owned(), manifest_bytes)?,
        active_checkpoint: manifest.plan.checkpoint,
        included_ids: manifest
            .plan
            .units
            .into_iter()
            .map(|unit| unit.id)
            .collect(),
    })
}

fn resolve_context(request_path: &Path) -> Result<ContextResult, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let arguments = [
        OsString::from("resolve-context"),
        request_path.as_os_str().to_owned(),
    ];
    let code = methexis::run(arguments, &mut stdout, &mut stderr)
        .map_err(|error| format!("cannot run Methexis ContextBuild: {error}"))?;
    if code != ExitCode::SUCCESS {
        return Err(format!(
            "Methexis ContextBuild failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    serde_json::from_slice(&stdout)
        .map_err(|error| format!("invalid Methexis ContextBuild result: {error}"))
}

fn capture_diff(repository: &Path, base: &str, candidate: &str) -> Result<Vec<u8>, String> {
    trusted_git_bytes(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )
}

fn capture_authorities(
    repository: &Path,
    candidate: &str,
    paths: &[String],
) -> Result<Vec<Captured>, String> {
    let paths = sorted_unique(paths, "repository authority path")?;
    paths
        .into_iter()
        .map(|path| {
            require_repository_path(&path)?;
            let listing = trusted_git_bytes(
                repository,
                &["ls-tree", "-z", "--full-tree", candidate, "--", &path],
            )?;
            let entry = listing
                .strip_suffix(&[0])
                .ok_or_else(|| format!("authority `{path}` has no exact Git tree entry"))?;
            let separator = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| format!("authority `{path}` has an invalid Git tree entry"))?;
            let (header, listed_path) = (&entry[..separator], &entry[separator + 1..]);
            if listed_path != path.as_bytes() {
                return Err(format!("authority `{path}` did not resolve exactly"));
            }
            let header = std::str::from_utf8(header)
                .map_err(|error| format!("invalid authority tree entry: {error}"))?;
            let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || fields[0] != "100644" || fields[1] != "blob" {
                return Err(format!(
                    "authority `{path}` must be a non-executable regular Git blob"
                ));
            }
            let bytes = trusted_git_bytes(repository, &["cat-file", "blob", fields[2]])?;
            captured(path, bytes)
        })
        .collect()
}

fn capture_validation(
    repository: &Path,
    requests: &[EvidenceRequest],
) -> Result<Vec<NamedCaptured>, String> {
    let mut names = BTreeSet::new();
    let mut captured_inputs = Vec::new();
    for request in requests {
        if request.name.trim().is_empty() || !names.insert(request.name.clone()) {
            return Err("validation evidence names must be non-empty and unique".to_owned());
        }
        let path = resolve_input_path(repository, &request.path);
        let bytes = bounded_file::read_regular(&path, MAX_INPUT_BYTES, "validation evidence")?;
        captured_inputs.push(NamedCaptured {
            name: request.name.clone(),
            artifact: captured(path.to_string_lossy().into_owned(), bytes)?,
        });
    }
    captured_inputs.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(captured_inputs)
}

fn build_plan(inputs: &Inputs) -> ReviewPlan {
    ReviewPlan {
        schema: PLAN_SCHEMA.to_owned(),
        base_commit: inputs.base_commit.clone(),
        candidate_commit: inputs.candidate_commit.clone(),
        diff_hash: inputs.diff.hash.clone(),
        trusted_commit: inputs.context.result.trusted_commit.clone(),
        active_checkpoint: inputs.context.active_checkpoint.clone(),
        context_build_id: inputs.context.result.build_id.clone(),
        context_request: semantic_input(&inputs.context.request),
        context: semantic_input(&inputs.context.context),
        context_manifest: semantic_input(&inputs.context.manifest),
        required_knowledge_ids: inputs.required_knowledge_ids.clone(),
        repository_authorities: inputs
            .authorities
            .iter()
            .map(|input| SemanticInput {
                path: input.path.clone(),
                hash: input.hash.clone(),
            })
            .collect(),
        slice_contract: semantic_input(&inputs.slice_contract),
        validation_evidence: inputs
            .validation
            .iter()
            .map(|input| NamedSemanticInput {
                name: input.name.clone(),
                path: input.artifact.path.clone(),
                hash: input.artifact.hash.clone(),
            })
            .collect(),
        review_lenses: inputs.lenses.clone(),
        review_questions: inputs.questions.clone(),
        delivery_profile: delivery_profile(),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        tokenizer_compiler: TOKENIZER_COMPILER.to_owned(),
        max_managed_payload_tokens: inputs.max_tokens,
    }
}

fn render_packet(review_id: &str, plan: &ReviewPlan, inputs: &Inputs) -> Result<Vec<u8>, String> {
    let mut packet = PREAMBLE.as_bytes().to_vec();
    let plan_bytes = serde_json::to_vec_pretty(plan).expect("closed review plan serializes");
    append_section(&mut packet, "review_plan", review_id, "", &plan_bytes)?;
    append_section(
        &mut packet,
        "context_request",
        "context-request",
        &inputs.context.request.path,
        &inputs.context.request.bytes,
    )?;
    append_section(
        &mut packet,
        "context_manifest",
        &inputs.context.result.build_id,
        &inputs.context.manifest.path,
        &inputs.context.manifest.bytes,
    )?;
    append_section(
        &mut packet,
        "context",
        &inputs.context.result.build_id,
        &inputs.context.context.path,
        &inputs.context.context.bytes,
    )?;
    for authority in &inputs.authorities {
        append_section(
            &mut packet,
            "repository_authority",
            &authority.path,
            &authority.path,
            &authority.bytes,
        )?;
    }
    append_section(
        &mut packet,
        "slice_contract",
        "slice-contract",
        &inputs.slice_contract.path,
        &inputs.slice_contract.bytes,
    )?;
    for evidence in &inputs.validation {
        append_section(
            &mut packet,
            "validation_evidence",
            &evidence.name,
            &evidence.artifact.path,
            &evidence.artifact.bytes,
        )?;
    }
    let instructions = serde_json::to_vec_pretty(&serde_json::json!({
        "review_lenses": inputs.lenses,
        "review_questions": inputs.questions,
    }))
    .expect("review instructions serialize");
    append_section(
        &mut packet,
        "review_instructions",
        "requested-review",
        "",
        &instructions,
    )?;
    append_section(
        &mut packet,
        "git_diff",
        "base-to-candidate",
        &inputs.diff.path,
        &inputs.diff.bytes,
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
        .map_err(|_| format!("review section `{name}` is not UTF-8 model-visible text"))?;
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
    review_id: String,
    plan: ReviewPlan,
    inputs: &Inputs,
    packet_hash: String,
    managed_payload_tokens: usize,
) -> Manifest {
    Manifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        review_id,
        plan,
        inputs: ManifestInputs {
            context_request: artifact(&inputs.context.request),
            context: artifact(&inputs.context.context),
            context_manifest: artifact(&inputs.context.manifest),
            repository_authorities: inputs.authorities.iter().map(artifact).collect(),
            slice_contract: artifact(&inputs.slice_contract),
            validation_evidence: inputs
                .validation
                .iter()
                .map(|input| NamedArtifact {
                    name: input.name.clone(),
                    artifact: artifact(&input.artifact),
                })
                .collect(),
            diff: artifact(&inputs.diff),
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
    trusted_ensure_clean(
        repository,
        "candidate worktree",
        "returning a review packet",
    )?;
    if trusted_resolve_commit(repository, "HEAD")? != inputs.candidate_commit {
        return Err("candidate HEAD changed during review packet construction".to_owned());
    }
    if trusted_resolve_commit(repository, "refs/heads/develop")?
        != inputs.context.result.trusted_commit
    {
        return Err("trusted integration changed during review packet construction".to_owned());
    }
    if capture_diff(repository, &inputs.base_commit, &inputs.candidate_commit)? != inputs.diff.bytes
    {
        return Err("base-to-candidate diff changed during review packet construction".to_owned());
    }
    let authorities = capture_authorities(
        repository,
        &inputs.candidate_commit,
        &request.repository_authority_paths,
    )?;
    require_captures(&authorities, &inputs.authorities, "repository authority")?;
    let contract_path = resolve_input_path(repository, &request.slice_contract_path);
    require_current_file(&contract_path, &inputs.slice_contract, "Slice contract")?;
    let bound = slice_contract::trusted_bound_slice(repository)?;
    let canonical_contract = std::fs::canonicalize(&contract_path)
        .map_err(|error| format!("cannot resolve Slice contract: {error}"))?;
    if bound.contract_path != canonical_contract
        || bound.base != inputs.base_commit
        || bound.contract_id != inputs.slice_contract.hash
    {
        return Err("bound Slice contract identity changed".to_owned());
    }
    let validation = capture_validation(repository, &request.validation_evidence)?;
    require_named_captures(&validation, &inputs.validation)?;
    let context_request_path = resolve_input_path(repository, &request.context_request_path);
    require_current_file(
        &context_request_path,
        &inputs.context.request,
        "ContextBuild request",
    )?;
    let current = capture_context(repository, &context_request_path)?;
    if current.result.trusted_commit != inputs.context.result.trusted_commit
        || current.result.build_id != inputs.context.result.build_id
        || current.result.context != inputs.context.result.context
        || current.result.manifest != inputs.context.result.manifest
        || current.active_checkpoint != inputs.context.active_checkpoint
        || current.included_ids != inputs.context.included_ids
        || current.context.bytes != inputs.context.context.bytes
        || current.manifest.bytes != inputs.context.manifest.bytes
    {
        return Err("ContextBuild identity, freshness, or artifact bytes changed".to_owned());
    }
    if delivery_profile_bytes() != inputs.delivery_profile_bytes {
        return Err("delivery profile bytes changed during review packet construction".to_owned());
    }
    Ok(())
}

fn require_current_file(path: &Path, expected: &Captured, label: &str) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, MAX_INPUT_BYTES, label)?;
    if digest(&bytes) == expected.hash && bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during review packet construction"))
    }
}

fn require_captures(actual: &[Captured], expected: &[Captured], label: &str) -> Result<(), String> {
    if actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(left, right)| {
            left.path == right.path && left.hash == right.hash && left.bytes == right.bytes
        })
    {
        Ok(())
    } else {
        Err(format!(
            "{label} inputs changed during review packet construction"
        ))
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
        Err("validation evidence changed during review packet construction".to_owned())
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
        .map_err(|_| "canonical review packet is not UTF-8".to_owned())?;
    Ok(tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(text)
        .len())
}

fn require_budget(actual: usize, maximum: usize) -> Result<(), String> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(format!(
            "managed payload requires {actual} tokens but the budget is {maximum}; no content was truncated"
        ))
    }
}

fn captured(path: String, bytes: Vec<u8>) -> Result<Captured, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "review input `{path}` exceeds the {MAX_INPUT_BYTES}-byte limit"
        ));
    }
    std::str::from_utf8(&bytes)
        .map_err(|_| format!("review input `{path}` is not UTF-8 model-visible text"))?;
    Ok(Captured {
        path,
        hash: digest(&bytes),
        bytes,
    })
}

fn semantic_input(input: &Captured) -> SemanticInput {
    SemanticInput {
        path: input.path.clone(),
        hash: input.hash.clone(),
    }
}

fn require_repository_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || matches!(component, Component::Normal(value) if value.to_string_lossy().contains(':'))
        })
    {
        return Err("repository authority paths must be safe relative paths".to_owned());
    }
    Ok(())
}

fn require_hash(expected: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    let actual = digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}

fn trusted_resolve_commit(repository: &Path, reference: &str) -> Result<String, String> {
    let value = trusted_git_text(
        repository,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )?;
    let value = value.trim().to_owned();
    require_commit(&value, "resolved commit")?;
    Ok(value)
}

fn trusted_git_succeeds(repository: &Path, arguments: &[&str]) -> Result<bool, String> {
    git::trusted_succeeds_in(repository, arguments)
}

fn trusted_git_text(repository: &Path, arguments: &[&str]) -> Result<String, String> {
    git::trusted_output_in(repository, arguments)
}

fn trusted_git_bytes(repository: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    git::trusted_output_bytes_in(repository, arguments)
}

fn trusted_repository_root(directory: &Path) -> Result<PathBuf, String> {
    let root = trusted_git_text(directory, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim();
    if root.is_empty() {
        return Err("trusted Git returned an empty repository root".to_owned());
    }
    Ok(PathBuf::from(root))
}

fn trusted_ensure_clean(repository: &Path, label: &str, operation: &str) -> Result<(), String> {
    let status = trusted_git_bytes(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} must be clean before {operation}"))
    }
}

fn expected_slice_ref(base_ref: &str, slice: &str) -> Result<String, String> {
    if base_ref == "refs/heads/develop" {
        return Ok(format!("refs/heads/slice/direct/{slice}"));
    }
    let wave = base_ref
        .strip_prefix("refs/heads/wave/")
        .filter(|wave| !wave.is_empty() && !wave.contains('/'))
        .ok_or_else(|| format!("unsupported Slice integration ref `{base_ref}`"))?;
    Ok(format!("refs/heads/slice/{wave}/{slice}"))
}
