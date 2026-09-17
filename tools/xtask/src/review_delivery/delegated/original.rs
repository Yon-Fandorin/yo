use std::path::Path;

use super::{
    super::{
        DIAGNOSTIC_LIMIT, DeliveryPolicy, REQUEST_LIMIT, REVIEW_RESULT_LIMIT,
        admission::evaluate_host_admission,
        artifact::{
            artifact, canonical_json, combine_failures, publish_claim, publish_exact,
            publish_provider_usage, require_exact_file_hash, sha256_file,
        },
        delegated_session::observe_host_session,
        model::{DELEGATED_REQUEST_SCHEMA_V1_ALPHA2, DelegatedRequest},
        process::{execute_delegated_once, exit_label, process_outcome},
        usage::{UsageBinding, UsageTarget},
        workspace::{
            build_current_yo, delivery_output_directory, integration_worktree,
            require_empty_directory, require_integration_state, shared_path,
        },
    },
    claims,
};
use crate::review_egress::{self, AuthorizedHostDelivery};

pub(in crate::review_delivery) fn run_original(
    repository: &Path,
    request: DelegatedRequest,
    policy: DeliveryPolicy,
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
        delivery_output_directory(repository, &request.output_directory, policy.prepare_output)?;

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
    let execution_isolation = final_admission.delegated_execution_isolation();
    super::super::runner_capability::require(&integration, &authorized.host, execution_isolation)?;
    require_integration_state(&integration, &authorized.trusted_commit)?;
    require_empty_directory(&output_directory)?;

    let claim = claims::original_claim(
        &authorized,
        require_state_readiness,
        execution_isolation,
        &yo_binary_hash,
        &request.admission_request_hash,
    );
    let claim_path = output_directory.join("claim.json");
    publish_claim(&claim_path, &canonical_json(&claim)?)?;

    let capture = execute_delegated_once(
        &yo_binary,
        &integration,
        &output_directory,
        &authorized,
        execution_isolation,
    );
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
    let (provider_usage, usage_failure) = if policy.bind_usage && observation.failure.is_none() {
        publish_provider_usage(
            &output_directory.join("sessions"),
            &output_directory,
            UsageBinding {
                review_id: authorized.review_id.clone(),
                packet_hash: authorized.packet_hash.clone(),
                packet_managed_tokens: authorized.managed_payload_tokens,
                request_id: observation
                    .host_request_id
                    .clone()
                    .expect("successful delegated observation has one host request"),
                session_id: observation
                    .session_id
                    .clone()
                    .expect("successful delegated observation has one Session"),
                turn_id: observation
                    .turn_id
                    .expect("successful delegated observation has one request turn"),
                target: UsageTarget::DelegatedHost {
                    host: authorized.host.clone(),
                },
            },
        )
        .map(|artifact| (Some(artifact), None))
        .unwrap_or_else(|error| (None, Some(error)))
    } else {
        (None, None)
    };
    let failure = [
        capture.failure.clone(),
        status_failure,
        observation.failure.clone(),
        review_publication_failure,
        diagnostic_publication_failure,
        usage_failure,
    ]
    .into_iter()
    .fold(None, combine_failures);
    let completed = failure.is_none();
    let outcome = claims::original_outcome(
        &authorized.request_id,
        completed,
        process_outcome(capture.status.as_ref()),
        observation.session_id.clone(),
        observation.host_request_count,
        observation.host_request_id.clone(),
        artifact(&review_path, &capture.stdout, review_artifact.published),
        artifact(
            &diagnostic_path,
            &capture.stderr,
            diagnostic_artifact.published,
        ),
        failure.clone(),
    );
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
    let receipt = claims::receipt(
        &authorized,
        execution_isolation,
        session_id,
        host_request_id,
    );
    let receipt_path = output_directory.join("delivery.json");
    let receipt_bytes = canonical_json(&receipt)?;
    publish_exact(
        &receipt_path,
        &receipt_bytes,
        REQUEST_LIMIT,
        "delegated external review delivery receipt",
    )?;
    let receipt_artifact = artifact(&receipt_path, &receipt_bytes, true);
    let result = claims::original_result(
        policy.bind_usage,
        require_state_readiness,
        &authorized,
        session_id,
        host_request_id,
        review_artifact,
        diagnostic_artifact,
        outcome_artifact,
        receipt_artifact,
        provider_usage,
    );
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode delegated delivery result: {error}"))?
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
