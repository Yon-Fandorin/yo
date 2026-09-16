use std::path::Path;

use super::{
    DIAGNOSTIC_LIMIT, DeliveryPolicy, REQUEST_LIMIT, REVIEW_RESULT_LIMIT,
    admission::{AdmissionReference, evaluate_admission},
    artifact::{
        artifact, canonical_json, combine_failures, publish_claim, publish_exact,
        publish_provider_usage, route, sha256_file,
    },
    model::{
        CONTINUATION_CLAIM_SCHEMA, CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2,
        CONTINUATION_OUTCOME_SCHEMA, CONTINUATION_RESULT_SCHEMA,
        CONTINUATION_RESULT_SCHEMA_V1_ALPHA2, CONTINUATION_RESULT_SCHEMA_V1_ALPHA3,
        ContinuationClaim, ContinuationDeliveryOutcome, ContinuationRequest,
        ContinuationResultDocument, DELIVERY_RECEIPT_SCHEMA, DeliveryReceipt,
    },
    process::{execute_continuation_once, exit_label, process_outcome},
    session::observe_continuation,
    usage::{UsageBinding, UsageTarget},
    workspace::{
        build_current_yo, delivery_output_directory, integration_worktree, require_empty_directory,
        require_integration_state, shared_path,
    },
};
use crate::review_continuation_preflight;

pub(super) fn run_continuation(
    repository: &Path,
    request: ContinuationRequest,
    admission: Option<AdmissionReference>,
    policy: DeliveryPolicy,
) -> Result<(), String> {
    let preflight_request_path = shared_path(repository, &request.preflight_request_path)?;
    require_exact_file_hash(
        &preflight_request_path,
        &request.preflight_request_hash,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let output_directory =
        delivery_output_directory(repository, &request.output_directory, policy.prepare_output)?;

    let initial = review_continuation_preflight::evaluate(repository, &preflight_request_path)?;
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
    let verified = review_continuation_preflight::evaluate(repository, &preflight_request_path)?;
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
    let (provider_usage, usage_failure) = if policy.bind_usage && observation.failure.is_none() {
        publish_provider_usage(
            &verified.session_root,
            &output_directory,
            UsageBinding {
                review_id: authorized.review_id.clone(),
                packet_hash: authorized.packet_hash.clone(),
                packet_managed_tokens: authorized.managed_payload_tokens,
                request_id: observation
                    .provider_request_id
                    .clone()
                    .expect("successful continuation has one Provider request"),
                session_id: session_id.to_owned(),
                turn_id: observation
                    .turn_id
                    .expect("successful continuation has one request turn"),
                target: UsageTarget::ManagedModel {
                    provider: authorized.provider.clone(),
                    account: authorized.account.clone(),
                    model: authorized.model.clone(),
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
        schema: if policy.bind_usage {
            CONTINUATION_RESULT_SCHEMA_V1_ALPHA3
        } else if admission.is_some() {
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
        provider_usage,
    };
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode Slice review continuation delivery result: {error}")
        })?
    );
    Ok(())
}
