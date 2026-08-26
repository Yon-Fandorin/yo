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
    AdmittedContinuationRequest, AdmittedRequest, Artifact, CLAIM_SCHEMA, CLAIM_SCHEMA_V1_ALPHA2,
    CONTINUATION_CLAIM_SCHEMA, CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2, CONTINUATION_OUTCOME_SCHEMA,
    CONTINUATION_REQUEST_SCHEMA, CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2, CONTINUATION_RESULT_SCHEMA,
    CONTINUATION_RESULT_SCHEMA_V1_ALPHA2, Claim, ContinuationClaim, ContinuationDeliveryOutcome,
    ContinuationRequest, ContinuationResultDocument, DELIVERY_RECEIPT_SCHEMA, DeliveryOutcome,
    DeliveryReceipt, DeliveryRequest, OUTCOME_SCHEMA, REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2,
    RESULT_SCHEMA, RESULT_SCHEMA_V1_ALPHA2, Request, ResultDocument, Route,
};
use process::{execute_continuation_once, execute_once, exit_label, process_outcome};
use serde::Serialize;
use session::{observe_continuation, observe_session};
use sha2::{Digest, Sha256};

use crate::{
    bounded_file, git,
    review_egress::{self, AuthorizedDelivery},
    review_protocol::digest,
    review_target_admission::{self, Admission, ReviewTarget},
    slice_contract,
};

const REQUEST_LIMIT: usize = 64 * 1024;
const REVIEW_RESULT_LIMIT: usize = 4 * 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 256 * 1024;

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    match read_request(request_path)? {
        DeliveryRequest::Original(request) => run_original(repository, request, None),
        DeliveryRequest::AdmittedOriginal(request) => {
            let admission = AdmissionReference {
                path: request.admission_request_path,
                hash: request.admission_request_hash,
            };
            run_original(
                repository,
                Request {
                    schema: request.schema,
                    egress_request_path: request.egress_request_path,
                    egress_request_hash: request.egress_request_hash,
                    output_directory: request.output_directory,
                },
                Some(admission),
            )
        },
        DeliveryRequest::Continuation(request) => run_continuation(repository, request, None),
        DeliveryRequest::AdmittedContinuation(request) => {
            let admission = AdmissionReference {
                path: request.admission_request_path,
                hash: request.admission_request_hash,
            };
            run_continuation(
                repository,
                ContinuationRequest {
                    schema: request.schema,
                    preflight_request_path: request.preflight_request_path,
                    preflight_request_hash: request.preflight_request_hash,
                    output_directory: request.output_directory,
                },
                Some(admission),
            )
        },
    }
}

#[derive(Debug)]
struct AdmissionReference {
    path: String,
    hash: String,
}

fn run_original(
    repository: &Path,
    request: Request,
    admission: Option<AdmissionReference>,
) -> Result<(), String> {
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
    let initial_admission = admission
        .as_ref()
        .map(|reference| evaluate_admission(repository, reference, &initial))
        .transpose()?;
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
    let final_admission = admission
        .as_ref()
        .map(|reference| evaluate_admission(repository, reference, &authorized))
        .transpose()?;
    if final_admission != initial_admission {
        return Err("external review target admission changed while preparing delivery".to_owned());
    }
    let model_reference = managed_model_reference(&authorized)?;
    require_integration_state(&integration, &authorized.trusted_commit)?;
    require_empty_directory(&output_directory)?;
    let claim_path = output_directory.join("claim.json");
    let claim = Claim {
        schema: if admission.is_some() {
            CLAIM_SCHEMA_V1_ALPHA2
        } else {
            CLAIM_SCHEMA
        },
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
        admission_request_id: admission.as_ref().map(|reference| reference.hash.as_str()),
        target: final_admission.as_ref().map(|admission| &admission.target),
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
        schema: if admission.is_some() {
            RESULT_SCHEMA_V1_ALPHA2
        } else {
            RESULT_SCHEMA
        },
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

fn run_continuation(
    repository: &Path,
    request: ContinuationRequest,
    admission: Option<AdmissionReference>,
) -> Result<(), String> {
    let preflight_request_path = shared_path(repository, &request.preflight_request_path)?;
    require_exact_file_hash(
        &preflight_request_path,
        &request.preflight_request_hash,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let output_directory = output_directory(repository, &request.output_directory)?;
    require_empty_directory(&output_directory)?;

    let initial =
        crate::review_continuation_preflight::evaluate(repository, &preflight_request_path)?;
    let initial_admission = admission
        .as_ref()
        .map(|reference| evaluate_admission(repository, reference, &initial.delivery))
        .transpose()?;
    let integration = integration_worktree(repository, &initial.delivery.trusted_commit)?;
    let yo_binary = build_current_yo(&integration)?;
    let yo_binary_hash = sha256_file(&yo_binary)?;

    require_exact_file_hash(
        &preflight_request_path,
        &request.preflight_request_hash,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let verified =
        crate::review_continuation_preflight::evaluate(repository, &preflight_request_path)?;
    if verified != initial {
        return Err(
            "reviewer Session or continuation authority changed while preparing delivery"
                .to_owned(),
        );
    }
    let authorized = &verified.delivery;
    let final_admission = admission
        .as_ref()
        .map(|reference| evaluate_admission(repository, reference, authorized))
        .transpose()?;
    if final_admission != initial_admission {
        return Err(
            "external review target admission changed while preparing continuation delivery"
                .to_owned(),
        );
    }
    require_integration_state(&integration, &authorized.trusted_commit)?;
    require_empty_directory(&output_directory)?;
    let session_id = authorized
        .session_id
        .as_deref()
        .expect("continuation preflight requires a reviewer Session");
    let prior_provider_request_id = authorized
        .prior_provider_request_id
        .as_deref()
        .expect("continuation preflight requires a prior Provider request identity");
    let claim = ContinuationClaim {
        schema: if admission.is_some() {
            CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            CONTINUATION_CLAIM_SCHEMA
        },
        request_id: &authorized.request_id,
        preflight_request_id: &verified.preflight_request_id,
        authorization_id: &authorized.authorization_id,
        authority: &authorized.authority,
        review_id: &authorized.review_id,
        candidate_commit: &authorized.candidate_commit,
        integration_commit: &authorized.trusted_commit,
        packet_hash: &authorized.packet_hash,
        packet_bytes: authorized.packet_bytes.len(),
        managed_payload_tokens: authorized.managed_payload_tokens,
        route: route(authorized),
        session_mode: "resume",
        session_id,
        prior_provider_request_id,
        continuation_anchor_sequence: verified.continuation_anchor_sequence,
        binding_epoch: verified.binding_epoch,
        provider_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        second_provider: false,
        tool_execution: false,
        yo_binary_hash: &yo_binary_hash,
        admission_request_id: admission.as_ref().map(|reference| reference.hash.as_str()),
        target: final_admission.as_ref().map(|admission| &admission.target),
    };
    let claim_path = output_directory.join("claim.json");
    publish_claim(&claim_path, &canonical_json(&claim)?)?;

    let capture = execute_continuation_once(
        &yo_binary,
        &integration,
        &output_directory,
        &verified.session_root,
        session_id,
        authorized,
    );
    let observation = observe_continuation(
        &verified.session_root,
        &authorized.packet_bytes,
        authorized,
        verified.continuation_anchor_sequence,
        verified.binding_epoch,
    );
    let status_failure = capture.status.as_ref().and_then(|status| {
        (!status.success()).then(|| {
            format!(
                "current-develop yo continuation exited without success ({})",
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
    let outcome = ContinuationDeliveryOutcome {
        schema: CONTINUATION_OUTCOME_SCHEMA,
        request_id: authorized.request_id.clone(),
        preflight_request_id: verified.preflight_request_id.clone(),
        status: if failure.is_none() {
            "completed"
        } else {
            "failed"
        },
        process: process_outcome(capture.status.as_ref()),
        session_id: session_id.to_owned(),
        durable_provider_request_count: observation.provider_request_count,
        provider_request_id: observation.provider_request_id.clone(),
        continuation_anchor_sequence: observation.continuation_anchor_sequence,
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
        "external review continuation delivery outcome",
    )?;
    let outcome_artifact = artifact(&outcome_path, &outcome_bytes, true);

    if let Some(failure) = failure {
        return Err(format!(
            "external review continuation stopped after its immutable one-attempt claim: {failure}; inspect {}",
            outcome_path.display()
        ));
    }

    let provider_request_id = observation
        .provider_request_id
        .as_deref()
        .expect("a completed continuation has one new Provider request identity");
    let continuation_anchor_sequence = observation
        .continuation_anchor_sequence
        .expect("a completed continuation has a new durable anchor");
    let receipt = DeliveryReceipt {
        schema: DELIVERY_RECEIPT_SCHEMA,
        review_id: &authorized.review_id,
        packet_hash: &authorized.packet_hash,
        route: route(authorized),
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
    let result = ContinuationResultDocument {
        schema: if admission.is_some() {
            CONTINUATION_RESULT_SCHEMA_V1_ALPHA2
        } else {
            CONTINUATION_RESULT_SCHEMA
        },
        ok: true,
        status: "completed",
        next_action: "interpret_review",
        request_id: authorized.request_id.clone(),
        preflight_request_id: verified.preflight_request_id,
        review_id: authorized.review_id.clone(),
        candidate_commit: authorized.candidate_commit.clone(),
        integration_commit: authorized.trusted_commit.clone(),
        session_id: session_id.to_owned(),
        provider_request_id: provider_request_id.to_owned(),
        continuation_anchor_sequence,
        review_result: review_artifact,
        diagnostic: diagnostic_artifact,
        outcome: outcome_artifact,
        delivery_receipt: receipt_artifact,
    };
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode Slice review continuation delivery result: {error}")
        })?
    );
    Ok(())
}

fn read_request(path: &Path) -> Result<DeliveryRequest, String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, "Slice review delivery request")?;
    let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid Slice review delivery request {}: {error}",
            path.display()
        )
    })?;
    let schema = header
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Slice review delivery request has no string schema".to_owned())?;
    match schema {
        REQUEST_SCHEMA => {
            let request: Request = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid original review delivery request: {error}"))?;
            debug_assert_eq!(request.schema, REQUEST_SCHEMA);
            compact_path(&request.egress_request_path, "egress_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.egress_request_hash, "egress_request_hash")?;
            Ok(DeliveryRequest::Original(request))
        },
        REQUEST_SCHEMA_V1_ALPHA2 => {
            let request: AdmittedRequest = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid admitted original review delivery request: {error}")
            })?;
            debug_assert_eq!(request.schema, REQUEST_SCHEMA_V1_ALPHA2);
            compact_path(&request.egress_request_path, "egress_request_path")?;
            compact_path(&request.admission_request_path, "admission_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.egress_request_hash, "egress_request_hash")?;
            require_sha256(&request.admission_request_hash, "admission_request_hash")?;
            Ok(DeliveryRequest::AdmittedOriginal(request))
        },
        CONTINUATION_REQUEST_SCHEMA => {
            let request: ContinuationRequest = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid continuation review delivery request: {error}")
            })?;
            debug_assert_eq!(request.schema, CONTINUATION_REQUEST_SCHEMA);
            compact_path(&request.preflight_request_path, "preflight_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.preflight_request_hash, "preflight_request_hash")?;
            Ok(DeliveryRequest::Continuation(request))
        },
        CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2 => {
            let request: AdmittedContinuationRequest =
                serde_json::from_slice(&bytes).map_err(|error| {
                    format!("invalid admitted continuation review delivery request: {error}")
                })?;
            debug_assert_eq!(request.schema, CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2);
            compact_path(&request.preflight_request_path, "preflight_request_path")?;
            compact_path(&request.admission_request_path, "admission_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.preflight_request_hash, "preflight_request_hash")?;
            require_sha256(&request.admission_request_hash, "admission_request_hash")?;
            Ok(DeliveryRequest::AdmittedContinuation(request))
        },
        other => Err(format!(
            "unsupported Slice review delivery request schema `{other}`; expected `{REQUEST_SCHEMA}`, `{REQUEST_SCHEMA_V1_ALPHA2}`, `{CONTINUATION_REQUEST_SCHEMA}`, or `{CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2}`"
        )),
    }
}

fn evaluate_admission(
    repository: &Path,
    reference: &AdmissionReference,
    delivery: &AuthorizedDelivery,
) -> Result<Admission, String> {
    let path = shared_path(repository, &reference.path)?;
    require_exact_file_hash(
        &path,
        &reference.hash,
        REQUEST_LIMIT,
        "external review target admission request",
    )?;
    let admission = review_target_admission::evaluate(&path)?;
    let expected = ReviewTarget::managed(
        delivery.provider.clone(),
        delivery.account.clone(),
        delivery.model.clone(),
    );
    if admission.target != expected {
        return Err("review-target admission differs from the authorized managed route".to_owned());
    }
    if !admission.admitted() {
        return Err(format!(
            "external review target admission stopped before claim: {}",
            admission.availability_detail()
        ));
    }
    Ok(admission)
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
