use super::super::model::{
    Artifact, DELEGATED_CLAIM_SCHEMA, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2,
    DELEGATED_CLAIM_SCHEMA_V1_ALPHA3, DELEGATED_CONTINUATION_CLAIM_SCHEMA,
    DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2, DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA3,
    DELEGATED_CONTINUATION_OUTCOME_SCHEMA, DELEGATED_CONTINUATION_RESULT_SCHEMA,
    DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA2, DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA3,
    DELEGATED_DELIVERY_RECEIPT_SCHEMA, DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2,
    DELEGATED_OUTCOME_SCHEMA, DELEGATED_RESULT_SCHEMA, DELEGATED_RESULT_SCHEMA_V1_ALPHA2,
    DELEGATED_RESULT_SCHEMA_V1_ALPHA3, DelegatedClaim, DelegatedContinuationClaim,
    DelegatedContinuationDeliveryOutcome, DelegatedContinuationResultDocument,
    DelegatedDeliveryOutcome, DelegatedDeliveryReceipt, DelegatedResultDocument, DelegatedTarget,
    ProcessOutcome,
};
use crate::review_egress::AuthorizedHostDelivery;

pub(super) fn original_claim<'a>(
    delivery: &'a AuthorizedHostDelivery,
    require_state_readiness: bool,
    execution_isolation: Option<&'a str>,
    yo_binary_hash: &'a str,
    admission_request_hash: &'a str,
) -> DelegatedClaim<'a> {
    DelegatedClaim {
        schema: if execution_isolation.is_some() {
            DELEGATED_CLAIM_SCHEMA_V1_ALPHA3
        } else if require_state_readiness {
            DELEGATED_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CLAIM_SCHEMA
        },
        request_id: &delivery.request_id,
        authorization_id: &delivery.authorization_id,
        authority: &delivery.authority,
        review_id: &delivery.review_id,
        candidate_commit: &delivery.candidate_commit,
        integration_commit: &delivery.trusted_commit,
        packet_hash: &delivery.packet_hash,
        packet_bytes: delivery.packet_bytes.len(),
        managed_payload_tokens: delivery.managed_payload_tokens,
        target: target(delivery),
        execution_profile: &delivery.execution_profile,
        execution_isolation,
        session_mode: "fresh",
        host_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        target_switch: false,
        yo_binary_hash,
        admission_request_id: admission_request_hash,
    }
}

// 고정된 delegated wire envelope의 필드 순서와 소유 경계를 보존하려고 인자를 합치지 않습니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn continuation_claim<'a>(
    delivery: &'a AuthorizedHostDelivery,
    require_state_readiness: bool,
    execution_isolation: Option<&'a str>,
    preflight_request_id: &'a str,
    session_id: &'a str,
    prior_host_request_id: &'a str,
    continuation_anchor_sequence: u64,
    binding_epoch: u64,
    yo_binary_hash: &'a str,
    admission_request_hash: &'a str,
) -> DelegatedContinuationClaim<'a> {
    DelegatedContinuationClaim {
        schema: if execution_isolation.is_some() {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA3
        } else if require_state_readiness {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA
        },
        request_id: &delivery.request_id,
        preflight_request_id,
        authorization_id: &delivery.authorization_id,
        authority: &delivery.authority,
        review_id: &delivery.review_id,
        candidate_commit: &delivery.candidate_commit,
        integration_commit: &delivery.trusted_commit,
        packet_hash: &delivery.packet_hash,
        packet_bytes: delivery.packet_bytes.len(),
        managed_payload_tokens: delivery.managed_payload_tokens,
        target: target(delivery),
        execution_profile: &delivery.execution_profile,
        execution_isolation,
        session_mode: "resume",
        session_id,
        prior_host_request_id,
        continuation_anchor_sequence,
        binding_epoch,
        host_request_limit: 1,
        retries: 0,
        steer: 0,
        fallback: 0,
        target_switch: false,
        yo_binary_hash,
        admission_request_id: admission_request_hash,
    }
}

// 고정된 delegated wire envelope의 필드 순서와 소유 경계를 보존하려고 인자를 합치지 않습니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn original_outcome(
    request_id: &str,
    completed: bool,
    process: ProcessOutcome,
    session_id: Option<String>,
    host_request_count: usize,
    host_request_id: Option<String>,
    review_result: Artifact,
    diagnostic: Artifact,
    failure: Option<String>,
) -> DelegatedDeliveryOutcome {
    DelegatedDeliveryOutcome {
        schema: DELEGATED_OUTCOME_SCHEMA,
        request_id: request_id.to_owned(),
        status: if completed { "completed" } else { "failed" },
        process,
        session_id,
        durable_host_request_count: host_request_count,
        host_request_id,
        review_result,
        diagnostic,
        failure,
    }
}

// 고정된 delegated wire envelope의 필드 순서와 소유 경계를 보존하려고 인자를 합치지 않습니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn continuation_outcome(
    request_id: &str,
    preflight_request_id: &str,
    completed: bool,
    process: ProcessOutcome,
    session_id: String,
    host_request_count: usize,
    host_request_id: Option<String>,
    continuation_anchor_sequence: Option<u64>,
    review_result: Artifact,
    diagnostic: Artifact,
    failure: Option<String>,
) -> DelegatedContinuationDeliveryOutcome {
    DelegatedContinuationDeliveryOutcome {
        schema: DELEGATED_CONTINUATION_OUTCOME_SCHEMA,
        request_id: request_id.to_owned(),
        preflight_request_id: preflight_request_id.to_owned(),
        status: if completed { "completed" } else { "failed" },
        process,
        session_id,
        durable_host_request_count: host_request_count,
        host_request_id,
        continuation_anchor_sequence,
        review_result,
        diagnostic,
        failure,
    }
}

pub(super) fn receipt<'a>(
    delivery: &'a AuthorizedHostDelivery,
    execution_isolation: Option<&'a str>,
    session_id: &'a str,
    host_request_id: &'a str,
) -> DelegatedDeliveryReceipt<'a> {
    DelegatedDeliveryReceipt {
        schema: if execution_isolation.is_some() {
            DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_DELIVERY_RECEIPT_SCHEMA
        },
        review_id: &delivery.review_id,
        packet_hash: &delivery.packet_hash,
        target: target(delivery),
        execution_profile: &delivery.execution_profile,
        execution_isolation,
        session_id,
        host_request_id,
        host_request_count: 1,
    }
}

// 고정된 delegated wire envelope의 필드 순서와 소유 경계를 보존하려고 인자를 합치지 않습니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn original_result(
    bind_usage: bool,
    require_state_readiness: bool,
    delivery: &AuthorizedHostDelivery,
    session_id: &str,
    host_request_id: &str,
    review_result: Artifact,
    diagnostic: Artifact,
    outcome: Artifact,
    delivery_receipt: Artifact,
    provider_usage: Option<Artifact>,
) -> DelegatedResultDocument {
    DelegatedResultDocument {
        schema: if bind_usage {
            DELEGATED_RESULT_SCHEMA_V1_ALPHA3
        } else if require_state_readiness {
            DELEGATED_RESULT_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_RESULT_SCHEMA
        },
        ok: true,
        status: "completed",
        next_action: "interpret_review",
        request_id: delivery.request_id.clone(),
        review_id: delivery.review_id.clone(),
        candidate_commit: delivery.candidate_commit.clone(),
        integration_commit: delivery.trusted_commit.clone(),
        session_id: session_id.to_owned(),
        host_request_id: host_request_id.to_owned(),
        review_result,
        diagnostic,
        outcome,
        delivery_receipt,
        provider_usage,
    }
}

// 고정된 delegated wire envelope의 필드 순서와 소유 경계를 보존하려고 인자를 합치지 않습니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn continuation_result(
    bind_usage: bool,
    require_state_readiness: bool,
    delivery: &AuthorizedHostDelivery,
    preflight_request_id: &str,
    session_id: &str,
    host_request_id: &str,
    continuation_anchor_sequence: u64,
    review_result: Artifact,
    diagnostic: Artifact,
    outcome: Artifact,
    delivery_receipt: Artifact,
    provider_usage: Option<Artifact>,
) -> DelegatedContinuationResultDocument {
    DelegatedContinuationResultDocument {
        schema: if bind_usage {
            DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA3
        } else if require_state_readiness {
            DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CONTINUATION_RESULT_SCHEMA
        },
        ok: true,
        status: "completed",
        next_action: "interpret_review",
        request_id: delivery.request_id.clone(),
        preflight_request_id: preflight_request_id.to_owned(),
        review_id: delivery.review_id.clone(),
        candidate_commit: delivery.candidate_commit.clone(),
        integration_commit: delivery.trusted_commit.clone(),
        session_id: session_id.to_owned(),
        host_request_id: host_request_id.to_owned(),
        continuation_anchor_sequence,
        review_result,
        diagnostic,
        outcome,
        delivery_receipt,
        provider_usage,
    }
}

pub(in crate::review_delivery) fn target(delivery: &AuthorizedHostDelivery) -> DelegatedTarget<'_> {
    DelegatedTarget {
        kind: "delegated_host",
        host: &delivery.host,
    }
}
