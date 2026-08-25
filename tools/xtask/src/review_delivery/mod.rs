mod model;
mod process;
mod session;

#[cfg(test)]
mod tests;

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use model::{
    Artifact, CLAIM_SCHEMA, Claim, DELIVERY_RECEIPT_SCHEMA, DeliveryOutcome, DeliveryReceipt,
    OUTCOME_SCHEMA, REQUEST_SCHEMA, RESULT_SCHEMA, Request, ResultDocument, Route,
};
use process::{execute_once, exit_label, process_outcome};
use serde::Serialize;
use session::observe_session;
use sha2::{Digest, Sha256};

use crate::{
    bounded_file, git,
    review_egress::{self, AuthorizedDelivery},
    review_protocol::digest,
    slice_contract,
};

const REQUEST_LIMIT: usize = 64 * 1024;
const REVIEW_RESULT_LIMIT: usize = 4 * 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 256 * 1024;

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request = read_request(request_path)?;
    let egress_request_path = shared_path(repository, &request.egress_request_path)?;
    require_exact_file_hash(
        &egress_request_path,
        &request.egress_request_hash,
        REQUEST_LIMIT,
        "Slice review egress request",
    )?;
    let output_directory = output_directory(repository, &request.output_directory)?;
    require_empty_directory(&output_directory)?;

    let initial = review_egress::authorize_delivery(repository, &egress_request_path)?;
    require_original_fresh(&initial)?;
    let integration = integration_worktree(repository, &initial.trusted_commit)?;
    let yo_binary = build_current_yo(&integration)?;
    let yo_binary_hash = sha256_file(&yo_binary)?;

    require_exact_file_hash(
        &egress_request_path,
        &request.egress_request_hash,
        REQUEST_LIMIT,
        "Slice review egress request",
    )?;
    let authorized = review_egress::authorize_delivery(repository, &egress_request_path)?;
    if authorized != initial {
        return Err("external review authorization changed while preparing delivery".to_owned());
    }
    let model_reference = managed_model_reference(&authorized)?;
    require_integration_state(&integration, &authorized.trusted_commit)?;
    require_empty_directory(&output_directory)?;
    let claim_path = output_directory.join("claim.json");
    let claim = Claim {
        schema: CLAIM_SCHEMA,
        request_id: &authorized.request_id,
        authorization_id: &authorized.authorization_id,
        authority: &authorized.authority,
        review_id: &authorized.review_id,
        candidate_commit: &authorized.candidate_commit,
        integration_commit: &authorized.trusted_commit,
        packet_hash: &authorized.packet_hash,
        packet_bytes: authorized.packet_bytes.len(),
        managed_payload_tokens: authorized.managed_payload_tokens,
        route: route(&authorized),
        session_mode: "fresh",
        provider_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        second_provider: false,
        tool_execution: false,
        yo_binary_hash: &yo_binary_hash,
    };
    let claim_bytes = canonical_json(&claim)?;
    publish_claim(&claim_path, &claim_bytes)?;

    let capture = execute_once(
        &yo_binary,
        &integration,
        &output_directory,
        &model_reference,
        &authorized,
    );
    let observation = observe_session(
        &output_directory.join("sessions"),
        &authorized.packet_bytes,
        &authorized,
    );
    let status_failure = capture.status.as_ref().and_then(|status| {
        (!status.success()).then(|| {
            format!(
                "current-develop yo exited without success ({})",
                exit_label(status)
            )
        })
    });
    let review_path = output_directory.join("review.txt");
    let review_publication_failure = publish_exact(
        &review_path,
        &capture.stdout,
        REVIEW_RESULT_LIMIT,
        "review result",
    )
    .err();
    let diagnostic_path = output_directory.join("diagnostic.txt");
    let diagnostic_publication_failure = publish_exact(
        &diagnostic_path,
        &capture.stderr,
        DIAGNOSTIC_LIMIT,
        "review diagnostic",
    )
    .err();
    let review_artifact = artifact(
        &review_path,
        &capture.stdout,
        review_publication_failure.is_none(),
    );
    let diagnostic_artifact = artifact(
        &diagnostic_path,
        &capture.stderr,
        diagnostic_publication_failure.is_none(),
    );
    let failure = [
        capture.failure.clone(),
        status_failure,
        observation.failure.clone(),
        review_publication_failure,
        diagnostic_publication_failure,
    ]
    .into_iter()
    .fold(None, combine_failures);
    let completed = failure.is_none();
    let outcome = DeliveryOutcome {
        schema: OUTCOME_SCHEMA,
        request_id: authorized.request_id.clone(),
        status: if completed { "completed" } else { "failed" },
        process: process_outcome(capture.status.as_ref()),
        session_id: observation.session_id.clone(),
        durable_provider_request_count: observation.provider_request_count,
        provider_request_id: observation.provider_request_id.clone(),
        review_result: artifact(&review_path, &capture.stdout, review_artifact.published),
        diagnostic: artifact(
            &diagnostic_path,
            &capture.stderr,
            diagnostic_artifact.published,
        ),
        failure: failure.clone(),
    };
    let outcome_path = output_directory.join("outcome.json");
    let outcome_bytes = canonical_json(&outcome)?;
    publish_exact(
        &outcome_path,
        &outcome_bytes,
        REQUEST_LIMIT,
        "external review delivery outcome",
    )?;
    let outcome_artifact = artifact(&outcome_path, &outcome_bytes, true);

    if let Some(failure) = failure {
        return Err(format!(
            "external review delivery stopped after its immutable one-attempt claim: {failure}; inspect {}",
            outcome_path.display()
        ));
    }

    let session_id = observation
        .session_id
        .as_deref()
        .expect("a completed observation has one Session");
    let provider_request_id = observation
        .provider_request_id
        .as_deref()
        .expect("a completed observation has one Provider request identity");
    let receipt = DeliveryReceipt {
        schema: DELIVERY_RECEIPT_SCHEMA,
        review_id: &authorized.review_id,
        packet_hash: &authorized.packet_hash,
        route: route(&authorized),
        session_id,
        provider_request_id,
        provider_request_count: 1,
    };
    let receipt_path = output_directory.join("delivery.json");
    let receipt_bytes = canonical_json(&receipt)?;
    publish_exact(
        &receipt_path,
        &receipt_bytes,
        REQUEST_LIMIT,
        "external review delivery receipt",
    )?;
    let receipt_artifact = artifact(&receipt_path, &receipt_bytes, true);

    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        status: "completed",
        next_action: "interpret_review",
        request_id: authorized.request_id,
        review_id: authorized.review_id,
        candidate_commit: authorized.candidate_commit,
        integration_commit: authorized.trusted_commit,
        session_id: session_id.to_owned(),
        provider_request_id: provider_request_id.to_owned(),
        review_result: review_artifact,
        diagnostic: diagnostic_artifact,
        outcome: outcome_artifact,
        delivery_receipt: receipt_artifact,
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode Slice review delivery result: {error}"))?
    );
    Ok(())
}

fn read_request(path: &Path) -> Result<Request, String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, "Slice review delivery request")?;
    let request: Request = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid Slice review delivery request {}: {error}",
            path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice review delivery request schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.egress_request_path, "egress_request_path")?;
    compact_path(&request.output_directory, "output_directory")?;
    require_sha256(&request.egress_request_hash, "egress_request_hash")?;
    Ok(request)
}

fn require_original_fresh(delivery: &AuthorizedDelivery) -> Result<(), String> {
    if delivery.review_kind != "original" || !delivery.fresh_session {
        Err(
            "review-deliver v1alpha1 supports only one original packet in a fresh Session"
                .to_owned(),
        )
    } else {
        Ok(())
    }
}

fn managed_model_reference(delivery: &AuthorizedDelivery) -> Result<String, String> {
    if [
        delivery.provider.as_str(),
        delivery.account.as_str(),
        delivery.model.as_str(),
    ]
    .into_iter()
    .any(|part| part.contains(':'))
    {
        return Err(
            "review-deliver v1alpha1 managed route components must not contain `:`".to_owned(),
        );
    }
    Ok(format!(
        "{}:{}:{}",
        delivery.provider, delivery.account, delivery.model
    ))
}

fn output_directory(repository: &Path, requested: &str) -> Result<PathBuf, String> {
    let bound = slice_contract::trusted_bound_slice(repository)?;
    let root = common_workspace_root(repository)?;
    let coordination = root
        .join(".local-exclude")
        .join("coordination")
        .join(bound.slice);
    let coordination = fs::canonicalize(&coordination).map_err(|error| {
        format!(
            "cannot resolve Slice coordination directory {}: {error}",
            coordination.display()
        )
    })?;
    let requested = PathBuf::from(requested);
    let requested = if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    };
    let requested = fs::canonicalize(&requested).map_err(|error| {
        format!(
            "cannot resolve review delivery output directory {}: {error}",
            requested.display()
        )
    })?;
    if requested == coordination || !requested.starts_with(&coordination) {
        return Err(format!(
            "review delivery output directory must be a child of {}",
            coordination.display()
        ));
    }
    Ok(requested)
}

fn shared_path(repository: &Path, requested: &str) -> Result<PathBuf, String> {
    let requested = PathBuf::from(requested);
    if requested.is_absolute() {
        Ok(requested)
    } else {
        common_workspace_root(repository).map(|root| root.join(requested))
    }
}

fn common_workspace_root(repository: &Path) -> Result<PathBuf, String> {
    let common = git::trusted_output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common.trim());
    if common.file_name().and_then(|name| name.to_str()) != Some(".git") {
        return Err("trusted Git common directory is not the repository .git directory".to_owned());
    }
    common
        .parent()
        .map(Path::to_owned)
        .ok_or_else(|| "trusted Git common directory has no workspace parent".to_owned())
}

fn require_empty_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect output directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("review delivery output must be a real directory".to_owned());
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("cannot read output directory {}: {error}", path.display()))?;
    if entries
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "cannot inspect output directory {}: {error}",
                path.display()
            )
        })?
        .is_some()
    {
        return Err(
            "review delivery output directory must be empty before its one attempt".to_owned(),
        );
    }
    Ok(())
}

fn integration_worktree(repository: &Path, expected_commit: &str) -> Result<PathBuf, String> {
    let output = git::trusted_output_in(repository, &["worktree", "list", "--porcelain"])?;
    let branch = "refs/heads/develop";
    let mut matches = output
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut observed_branch = None;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(value));
                } else if let Some(value) = line.strip_prefix("branch ") {
                    observed_branch = Some(value);
                }
            }
            (observed_branch == Some(branch)).then_some(path).flatten()
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "expected exactly one checked-out develop integration worktree, found {}",
            matches.len()
        ));
    }
    let integration = fs::canonicalize(matches.remove(0))
        .map_err(|error| format!("cannot resolve develop integration worktree: {error}"))?;
    require_integration_state(&integration, expected_commit)?;
    Ok(integration)
}

fn require_integration_state(integration: &Path, expected_commit: &str) -> Result<(), String> {
    let head = git::trusted_output_in(integration, &["rev-parse", "HEAD"])?;
    if head.trim() != expected_commit {
        return Err(format!(
            "develop integration worktree changed: expected {expected_commit}, found {}",
            head.trim()
        ));
    }
    let status = git::trusted_output_in(
        integration,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    if !status.trim().is_empty() {
        return Err("develop integration worktree must be clean before review delivery".to_owned());
    }
    Ok(())
}

fn build_current_yo(integration: &Path) -> Result<PathBuf, String> {
    let status = Command::new("cargo")
        .args([
            "build", "--quiet", "--locked", "-p", "yo-cli", "--bin", "yo",
        ])
        .env_remove("CARGO_TARGET_DIR")
        .current_dir(integration)
        .status()
        .map_err(|error| format!("cannot start current-develop yo build: {error}"))?;
    if !status.success() {
        return Err(format!(
            "current-develop yo build failed ({}) before any delivery claim",
            exit_label(&status)
        ));
    }
    let binary = integration.join("target").join("debug").join("yo");
    let metadata = fs::symlink_metadata(&binary).map_err(|error| {
        format!(
            "cannot inspect current-develop yo binary {}: {error}",
            binary.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("current-develop yo build did not produce a regular binary".to_owned());
    }
    Ok(binary)
}

fn route(delivery: &AuthorizedDelivery) -> Route<'_> {
    Route {
        provider: &delivery.provider,
        account: &delivery.account,
        model: &delivery.model,
    }
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot encode review delivery artifact: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn publish_exact(path: &Path, bytes: &[u8], limit: usize, label: &str) -> Result<(), String> {
    if bounded_file::publish_new_or_exact(path, bytes, limit, label)? {
        Ok(())
    } else {
        Err(format!(
            "{label} already exists at {}; refusing to reuse a completed delivery path",
            path.display()
        ))
    }
}

fn publish_claim(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bounded_file::publish_new_or_exact(
        path,
        bytes,
        REQUEST_LIMIT,
        "external review delivery claim",
    )? {
        Ok(())
    } else {
        Err(format!(
            "external review delivery is already claimed at {}; refusing another provider request",
            path.display()
        ))
    }
}

fn artifact(path: &Path, bytes: &[u8], published: bool) -> Artifact {
    Artifact {
        path: path.to_string_lossy().into_owned(),
        hash: digest(bytes),
        bytes: bytes.len(),
        published,
    }
}

fn require_exact_file_hash(
    path: &Path,
    expected: &str,
    limit: usize,
    label: &str,
) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, limit, label)?;
    let actual = digest(&bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot open current-develop yo binary: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash current-develop yo binary: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn compact_path(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!(
            "{label} must be a non-empty path of at most 4096 bytes"
        ))
    } else {
        Ok(())
    }
}

fn require_sha256(value: &str, label: &str) -> Result<(), String> {
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

fn combine_failures(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (None, None) => None,
        (Some(error), None) | (None, Some(error)) => Some(error),
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
    }
}
