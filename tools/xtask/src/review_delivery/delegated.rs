use std::path::Path;

use super::{
    DIAGNOSTIC_LIMIT, REQUEST_LIMIT, REVIEW_RESULT_LIMIT, artifact, build_current_yo,
    canonical_json, combine_failures,
    delegated_session::{observe_host_continuation, observe_host_session},
    evaluate_host_admission, exit_label, integration_worktree,
    model::{
        DELEGATED_CLAIM_SCHEMA, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2,
        DELEGATED_CONTINUATION_CLAIM_SCHEMA, DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2,
        DELEGATED_CONTINUATION_OUTCOME_SCHEMA, DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2,
        DELEGATED_CONTINUATION_RESULT_SCHEMA, DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA2,
        DELEGATED_DELIVERY_RECEIPT_SCHEMA, DELEGATED_OUTCOME_SCHEMA,
        DELEGATED_REQUEST_SCHEMA_V1_ALPHA2, DELEGATED_RESULT_SCHEMA,
        DELEGATED_RESULT_SCHEMA_V1_ALPHA2, DelegatedClaim, DelegatedContinuationClaim,
        DelegatedContinuationDeliveryOutcome, DelegatedContinuationRequest,
        DelegatedContinuationResultDocument, DelegatedDeliveryOutcome, DelegatedDeliveryReceipt,
        DelegatedRequest, DelegatedResultDocument, DelegatedTarget,
    },
    process::{execute_delegated_continuation_once, execute_delegated_once},
    process_outcome, publish_claim, publish_exact, require_empty_directory,
    require_exact_file_hash, require_integration_state, sha256_file, shared_path,
};
use crate::review_egress::{self, AuthorizedHostDelivery};

pub(super) fn run_original(
    repository: &Path,
    request: DelegatedRequest,
    prepare_output: bool,
) -> Result<(), String> {
    let require_state_readiness = request.schema == DELEGATED_REQUEST_SCHEMA_V1_ALPHA2;
    let egress_request_path = shared_path(repository, &request.egress_request_path)?;
    require_exact_file_hash(
        &egress_request_path,
        &request.egress_request_hash,
        REQUEST_LIMIT,
        "delegated Slice review egress request",
    )?;
    let output_directory =
        super::delivery_output_directory(repository, &request.output_directory, prepare_output)?;

    let initial = review_egress::authorize_host_delivery(repository, &egress_request_path)?;
    require_original_fresh(&initial)?;
    let initial_admission = evaluate_host_admission(
        repository,
        &request.admission_request_path,
        &request.admission_request_hash,
        &initial,
        require_state_readiness,
    )?;
    let integration = integration_worktree(repository, &initial.trusted_commit)?;
    let yo_binary = build_current_yo(&integration)?;
    let yo_binary_hash = sha256_file(&yo_binary)?;

    require_exact_file_hash(
        &egress_request_path,
        &request.egress_request_hash,
        REQUEST_LIMIT,
        "delegated Slice review egress request",
    )?;
    let authorized = review_egress::authorize_host_delivery(repository, &egress_request_path)?;
    if authorized != initial {
        return Err("delegated review authorization changed while preparing delivery".to_owned());
    }
    let final_admission = evaluate_host_admission(
        repository,
        &request.admission_request_path,
        &request.admission_request_hash,
        &authorized,
        require_state_readiness,
    )?;
    if final_admission != initial_admission {
        return Err(
            "delegated review target admission changed while preparing delivery".to_owned(),
        );
    }
    require_integration_state(&integration, &authorized.trusted_commit)?;
    require_empty_directory(&output_directory)?;

    let claim = DelegatedClaim {
        schema: if require_state_readiness {
            DELEGATED_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CLAIM_SCHEMA
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
        target: target(&authorized),
        execution_profile: &authorized.execution_profile,
        session_mode: "fresh",
        host_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        target_switch: false,
        yo_binary_hash: &yo_binary_hash,
        admission_request_id: &request.admission_request_hash,
    };
    let claim_path = output_directory.join("claim.json");
    publish_claim(&claim_path, &canonical_json(&claim)?)?;

    let capture = execute_delegated_once(&yo_binary, &integration, &output_directory, &authorized);
    let observation = observe_host_session(
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
    let outcome = DelegatedDeliveryOutcome {
        schema: DELEGATED_OUTCOME_SCHEMA,
        request_id: authorized.request_id.clone(),
        status: if completed { "completed" } else { "failed" },
        process: process_outcome(capture.status.as_ref()),
        session_id: observation.session_id.clone(),
        durable_host_request_count: observation.host_request_count,
        host_request_id: observation.host_request_id.clone(),
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
        "delegated external review delivery outcome",
    )?;
    let outcome_artifact = artifact(&outcome_path, &outcome_bytes, true);
    if let Some(failure) = failure {
        return Err(format!(
            "delegated review delivery stopped after its immutable one-attempt claim: {failure}; inspect {}",
            outcome_path.display()
        ));
    }

    let session_id = observation
        .session_id
        .as_deref()
        .expect("completed delegated observation has one Session");
    let host_request_id = observation
        .host_request_id
        .as_deref()
        .expect("completed delegated observation has one host request identity");
    let receipt = DelegatedDeliveryReceipt {
        schema: DELEGATED_DELIVERY_RECEIPT_SCHEMA,
        review_id: &authorized.review_id,
        packet_hash: &authorized.packet_hash,
        target: target(&authorized),
        execution_profile: &authorized.execution_profile,
        session_id,
        host_request_id,
        host_request_count: 1,
    };
    let receipt_path = output_directory.join("delivery.json");
    let receipt_bytes = canonical_json(&receipt)?;
    publish_exact(
        &receipt_path,
        &receipt_bytes,
        REQUEST_LIMIT,
        "delegated external review delivery receipt",
    )?;
    let receipt_artifact = artifact(&receipt_path, &receipt_bytes, true);
    let result = DelegatedResultDocument {
        schema: if require_state_readiness {
            DELEGATED_RESULT_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_RESULT_SCHEMA
        },
        ok: true,
        status: "completed",
        next_action: "interpret_review",
        request_id: authorized.request_id,
        review_id: authorized.review_id,
        candidate_commit: authorized.candidate_commit,
        integration_commit: authorized.trusted_commit,
        session_id: session_id.to_owned(),
        host_request_id: host_request_id.to_owned(),
        review_result: review_artifact,
        diagnostic: diagnostic_artifact,
        outcome: outcome_artifact,
        delivery_receipt: receipt_artifact,
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode delegated delivery result: {error}"))?
    );
    Ok(())
}

pub(super) fn run_continuation(
    repository: &Path,
    request: DelegatedContinuationRequest,
    prepare_output: bool,
) -> Result<(), String> {
    let require_state_readiness = request.schema == DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2;
    let preflight_path = shared_path(repository, &request.preflight_request_path)?;
    require_exact_file_hash(
        &preflight_path,
        &request.preflight_request_hash,
        REQUEST_LIMIT,
        "delegated continuation preflight request",
    )?;
    let output_directory =
        super::delivery_output_directory(repository, &request.output_directory, prepare_output)?;

    let initial =
        crate::review_continuation_preflight::evaluate_delegated(repository, &preflight_path)?;
    let initial_admission = evaluate_host_admission(
        repository,
        &request.admission_request_path,
        &request.admission_request_hash,
        &initial.delivery,
        require_state_readiness,
    )?;
    let integration = integration_worktree(repository, &initial.delivery.trusted_commit)?;
    let yo_binary = build_current_yo(&integration)?;
    let yo_binary_hash = sha256_file(&yo_binary)?;

    require_exact_file_hash(
        &preflight_path,
        &request.preflight_request_hash,
        REQUEST_LIMIT,
        "delegated continuation preflight request",
    )?;
    let verified =
        crate::review_continuation_preflight::evaluate_delegated(repository, &preflight_path)?;
    if verified != initial {
        return Err(
            "delegated reviewer Session or continuation authority changed while preparing delivery"
                .to_owned(),
        );
    }
    let authorized = &verified.delivery;
    let final_admission = evaluate_host_admission(
        repository,
        &request.admission_request_path,
        &request.admission_request_hash,
        authorized,
        require_state_readiness,
    )?;
    if final_admission != initial_admission {
        return Err(
            "delegated review target admission changed while preparing continuation".to_owned(),
        );
    }
    let session_id = authorized
        .session_id
        .as_deref()
        .ok_or_else(|| "delegated continuation authorization has no Session".to_owned())?;
    let prior_host_request_id = authorized
        .prior_host_request_id
        .as_deref()
        .ok_or_else(|| "delegated continuation has no prior host request identity".to_owned())?;
    require_integration_state(&integration, &authorized.trusted_commit)?;
    require_empty_directory(&output_directory)?;

    let claim = DelegatedContinuationClaim {
        schema: if require_state_readiness {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA
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
        target: target(authorized),
        execution_profile: &authorized.execution_profile,
        session_mode: "resume",
        session_id,
        prior_host_request_id,
        continuation_anchor_sequence: verified.continuation_anchor_sequence,
        binding_epoch: verified.binding_epoch,
        host_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        target_switch: false,
        yo_binary_hash: &yo_binary_hash,
        admission_request_id: &request.admission_request_hash,
    };
    let claim_path = output_directory.join("claim.json");
    publish_claim(&claim_path, &canonical_json(&claim)?)?;

    let capture = execute_delegated_continuation_once(
        &yo_binary,
        &integration,
        &output_directory,
        &verified.session_root,
        session_id,
        authorized,
    );
    let observation = observe_host_continuation(
        &verified.session_root,
        &authorized.packet_bytes,
        authorized,
        verified.continuation_anchor_sequence,
        verified.binding_epoch,
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
    let outcome = DelegatedContinuationDeliveryOutcome {
        schema: DELEGATED_CONTINUATION_OUTCOME_SCHEMA,
        request_id: authorized.request_id.clone(),
        preflight_request_id: verified.preflight_request_id.clone(),
        status: if completed { "completed" } else { "failed" },
        process: process_outcome(capture.status.as_ref()),
        session_id: session_id.to_owned(),
        durable_host_request_count: observation.host_request_count,
        host_request_id: observation.host_request_id.clone(),
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
        "delegated continuation delivery outcome",
    )?;
    let outcome_artifact = artifact(&outcome_path, &outcome_bytes, true);
    if let Some(failure) = failure {
        return Err(format!(
            "delegated continuation stopped after its immutable one-attempt claim: {failure}; inspect {}",
            outcome_path.display()
        ));
    }

    let host_request_id = observation
        .host_request_id
        .as_deref()
        .expect("completed delegated continuation has one host request identity");
    let continuation_anchor_sequence = observation
        .continuation_anchor_sequence
        .expect("completed delegated continuation has one new Anchor");
    let receipt = DelegatedDeliveryReceipt {
        schema: DELEGATED_DELIVERY_RECEIPT_SCHEMA,
        review_id: &authorized.review_id,
        packet_hash: &authorized.packet_hash,
        target: target(authorized),
        execution_profile: &authorized.execution_profile,
        session_id,
        host_request_id,
        host_request_count: 1,
    };
    let receipt_path = output_directory.join("delivery.json");
    let receipt_bytes = canonical_json(&receipt)?;
    publish_exact(
        &receipt_path,
        &receipt_bytes,
        REQUEST_LIMIT,
        "delegated external review delivery receipt",
    )?;
    let receipt_artifact = artifact(&receipt_path, &receipt_bytes, true);
    let result = DelegatedContinuationResultDocument {
        schema: if require_state_readiness {
            DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CONTINUATION_RESULT_SCHEMA
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
        host_request_id: host_request_id.to_owned(),
        continuation_anchor_sequence,
        review_result: review_artifact,
        diagnostic: diagnostic_artifact,
        outcome: outcome_artifact,
        delivery_receipt: receipt_artifact,
    };
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode delegated continuation delivery result: {error}")
        })?
    );
    Ok(())
}

fn require_original_fresh(delivery: &AuthorizedHostDelivery) -> Result<(), String> {
    if delivery.review_kind == "original" && delivery.fresh_session {
        Ok(())
    } else {
        Err("delegated delivery accepts only one original packet in a fresh Session".to_owned())
    }
}

pub(super) fn target(delivery: &AuthorizedHostDelivery) -> DelegatedTarget<'_> {
    DelegatedTarget {
        kind: "delegated_host",
        host: &delivery.host,
    }
}
