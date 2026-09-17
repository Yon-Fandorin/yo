use std::path::Path;

use super::{
    super::{
        DIAGNOSTIC_LIMIT, DeliveryPolicy, REQUEST_LIMIT, REVIEW_RESULT_LIMIT,
        admission::evaluate_host_admission,
        artifact::{
            artifact, canonical_json, combine_failures, publish_claim, publish_exact,
            publish_provider_usage, require_exact_file_hash, sha256_file,
        },
        delegated_session::observe_host_continuation,
        model::{DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2, DelegatedContinuationRequest},
        process::{execute_delegated_continuation_once, exit_label, process_outcome},
        usage::{UsageBinding, UsageTarget},
        workspace::{
            build_current_yo, delivery_output_directory, integration_worktree,
            require_empty_directory, require_integration_state, shared_path,
        },
    },
    claims,
};
use crate::{review_continuation_preflight, review_egress::AuthorizedHostDelivery};

pub(in crate::review_delivery) fn require_continuation_isolation(
    authorized: &AuthorizedHostDelivery,
    selected: Option<&str>,
) -> Result<(), String> {
    if authorized.prior_execution_isolation.as_deref() == selected {
        return Ok(());
    }
    Err(format!(
        "delegated continuation execution isolation changed from `{}` to `{}`; resume requires the exact prior physical isolation",
        authorized
            .prior_execution_isolation
            .as_deref()
            .unwrap_or("unrecorded"),
        selected.unwrap_or("unrecorded")
    ))
}

pub(in crate::review_delivery) fn run_continuation(
    repository: &Path,
    request: DelegatedContinuationRequest,
    policy: DeliveryPolicy,
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
        delivery_output_directory(repository, &request.output_directory, policy.prepare_output)?;

    let initial = review_continuation_preflight::evaluate_delegated(repository, &preflight_path)?;
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
    let verified = review_continuation_preflight::evaluate_delegated(repository, &preflight_path)?;
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
    let execution_isolation = final_admission.delegated_execution_isolation();
    require_continuation_isolation(authorized, execution_isolation)?;
    super::super::runner_capability::require(&integration, &authorized.host, execution_isolation)?;
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

    let claim = claims::continuation_claim(
        &authorized,
        require_state_readiness,
        execution_isolation,
        &verified.preflight_request_id,
        session_id,
        prior_host_request_id,
        verified.continuation_anchor_sequence,
        verified.binding_epoch,
        &yo_binary_hash,
        &request.admission_request_hash,
    );
    let claim_path = output_directory.join("claim.json");
    publish_claim(&claim_path, &canonical_json(&claim)?)?;

    let capture = execute_delegated_continuation_once(
        &yo_binary,
        &integration,
        &output_directory,
        &verified.session_root,
        session_id,
        authorized,
        execution_isolation,
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
    let (provider_usage, usage_failure) = if policy.bind_usage && observation.failure.is_none() {
        publish_provider_usage(
            &verified.session_root,
            &output_directory,
            UsageBinding {
                review_id: authorized.review_id.clone(),
                packet_hash: authorized.packet_hash.clone(),
                packet_managed_tokens: authorized.managed_payload_tokens,
                request_id: observation
                    .host_request_id
                    .clone()
                    .expect("successful delegated continuation has one host request"),
                session_id: session_id.to_owned(),
                turn_id: observation
                    .turn_id
                    .expect("successful delegated continuation has one request turn"),
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
    let outcome = claims::continuation_outcome(
        &authorized.request_id,
        &verified.preflight_request_id,
        completed,
        process_outcome(capture.status.as_ref()),
        session_id.to_owned(),
        observation.host_request_count,
        observation.host_request_id.clone(),
        observation.continuation_anchor_sequence,
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
    let receipt = claims::receipt(authorized, execution_isolation, session_id, host_request_id);
    let receipt_path = output_directory.join("delivery.json");
    let receipt_bytes = canonical_json(&receipt)?;
    publish_exact(
        &receipt_path,
        &receipt_bytes,
        REQUEST_LIMIT,
        "delegated external review delivery receipt",
    )?;
    let receipt_artifact = artifact(&receipt_path, &receipt_bytes, true);
    let result = claims::continuation_result(
        policy.bind_usage,
        require_state_readiness,
        authorized,
        &verified.preflight_request_id,
        session_id,
        host_request_id,
        continuation_anchor_sequence,
        review_artifact,
        diagnostic_artifact,
        outcome_artifact,
        receipt_artifact,
        provider_usage,
    );
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode delegated continuation delivery result: {error}")
        })?
    );
    Ok(())
}
