mod continuation;
mod delegated;
mod finalization;
mod original;
mod usage;
mod workspace;

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
use std::{
    fs,
    time::{Duration, Instant},
};

use super::{
    admission::{
        managed_model_reference, read_request, read_request_with_output_policy,
        require_original_fresh,
    },
    artifact::{canonical_json, combine_failures, publish_claim},
    delegated::require_continuation_isolation,
    model::{Artifact, CLAIM_SCHEMA, Claim, DeliveryRequest, ResultDocument, Route},
    process::{
        execute_continuation_once, execute_delegated_continuation_once, execute_delegated_once,
        execute_once, execute_once_with_timeout,
    },
    workspace::{prepare_output_directory_at, require_empty_directory, require_integration_state},
};
use crate::{
    git, grok_outer_sandbox,
    review_egress::{AuthorizedDelivery, AuthorizedHostDelivery},
    review_protocol::digest,
    review_session::provider_request_identity,
    test_support::TestRepository,
};

fn authorized() -> AuthorizedDelivery {
    AuthorizedDelivery {
        request_id: "sha256:request".to_owned(),
        authorization_id: "sha256:authorization".to_owned(),
        authority: "human/yon".to_owned(),
        review_kind: "original",
        review_id: "sha256:review".to_owned(),
        candidate_commit: "11".repeat(20),
        trusted_commit: "22".repeat(20),
        packet_hash: "sha256:packet".to_owned(),
        packet_bytes: b"review packet".to_vec(),
        managed_payload_tokens: 3,
        provider: "qwencloud".to_owned(),
        account: "default".to_owned(),
        model: "qwen3.8-max".to_owned(),
        fresh_session: true,
        session_id: None,
        prior_packet_hash: None,
        prior_provider_request_id: None,
    }
}

fn authorized_host() -> AuthorizedHostDelivery {
    AuthorizedHostDelivery {
        request_id: "sha256:request".to_owned(),
        authorization_id: "sha256:authorization".to_owned(),
        authority: "human/yon".to_owned(),
        review_kind: "original",
        review_id: "sha256:review".to_owned(),
        candidate_commit: "11".repeat(20),
        trusted_commit: "22".repeat(20),
        packet_hash: "sha256:packet".to_owned(),
        packet_bytes: b"review packet".to_vec(),
        managed_payload_tokens: 3,
        host: "codex".to_owned(),
        execution_profile: "yo.delegated-review-execution/v1alpha1".to_owned(),
        fresh_session: true,
        session_id: None,
        prior_packet_hash: None,
        prior_host_request_id: None,
        prior_execution_isolation: None,
    }
}
