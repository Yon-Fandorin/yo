mod authorization;
mod receipt;
mod target;

use std::path::Path;

pub(super) use authorization::{authorize, canonical_authorization_path, validate_authorization};
pub(super) use receipt::{parse_delivery_receipt, verify_completed_delivery_bytes};

use super::{
    model::{
        DELEGATED_REQUEST_SCHEMA, DELEGATED_RESULT_SCHEMA, DelegatedAuthorizationDocument,
        DelegatedDeliveryLimits, DelegatedRequest, DelegatedResultDocument, ManifestHeader,
        PacketResult, ReviewKind, Session,
    },
    validation::{
        AUTHORIZATION_LIMIT, DELIVERY_RECEIPT_LIMIT, MANIFEST_LIMIT, MAX_SESSION_ID_BYTES,
        PACKET_LIMIT, REQUEST_LIMIT, classify_review_kind, compact_path, compact_token,
        require_exact_hash, require_sha256,
    },
};
use crate::{
    bounded_file, review_delta,
    review_packet::VerifiedReview,
    review_protocol::{digest, resolve_input_path},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedHostDelivery {
    pub(crate) request_id: String,
    pub(crate) authorization_id: String,
    pub(crate) authority: String,
    pub(crate) review_kind: &'static str,
    pub(crate) review_id: String,
    pub(crate) candidate_commit: String,
    pub(crate) trusted_commit: String,
    pub(crate) packet_hash: String,
    pub(crate) packet_bytes: Vec<u8>,
    pub(crate) managed_payload_tokens: usize,
    pub(crate) host: String,
    pub(crate) execution_profile: String,
    pub(crate) fresh_session: bool,
    pub(crate) session_id: Option<String>,
    pub(crate) prior_packet_hash: Option<String>,
    pub(crate) prior_host_request_id: Option<String>,
    pub(crate) prior_execution_isolation: Option<String>,
}

pub(crate) fn authorize_host_delivery(
    repository: &Path,
    request_path: &Path,
) -> Result<AuthorizedHostDelivery, String> {
    authorize_with(repository, request_path, &|repository, manifest, hash| {
        review_delta::verify_chain_head(repository, manifest, hash, &mut Default::default(), 0)
    })
    .map(|(_, delivery)| delivery)
}

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let document = authorize_with(repository, request_path, &|repository, manifest, hash| {
        review_delta::verify_chain_head(repository, manifest, hash, &mut Default::default(), 0)
    })?
    .0;
    println!(
        "{}",
        serde_json::to_string(&document)
            .map_err(|error| format!("cannot encode delegated egress result: {error}"))?
    );
    Ok(())
}

fn authorize_with(
    repository: &Path,
    request_path: &Path,
    verify: &dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String>,
) -> Result<(DelegatedResultDocument, AuthorizedHostDelivery), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "delegated Slice review egress request",
    )?;
    let request: DelegatedRequest = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid delegated Slice review egress request {}: {error}",
            request_path.display()
        )
    })?;
    validate_request(&request)?;

    let authorization_path = canonical_authorization_path(repository)?;
    let authorization_bytes = bounded_file::read_regular(
        &authorization_path,
        AUTHORIZATION_LIMIT,
        "delegated external review authorization",
    )?;
    require_exact_hash(
        &request.authorization_hash,
        &authorization_bytes,
        "delegated external review authorization",
    )?;
    let authorization: DelegatedAuthorizationDocument =
        serde_json::from_slice(&authorization_bytes).map_err(|error| {
            format!(
                "invalid delegated external review authorization {}: {error}",
                authorization_path.display()
            )
        })?;
    validate_authorization(&authorization)?;

    let manifest_path = resolve_input_path(repository, &request.manifest_path);
    let manifest_bytes = bounded_file::read_regular(
        &manifest_path,
        MANIFEST_LIMIT,
        "published review-chain manifest",
    )?;
    require_exact_hash(
        &request.manifest_hash,
        &manifest_bytes,
        "published review-chain manifest",
    )?;
    let manifest: ManifestHeader = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid published review-chain manifest: {error}"))?;
    let classification = classify_review_kind(repository, &manifest)?;
    let verified = verify(repository, &manifest_path, &request.manifest_hash)?;

    let packet_path = resolve_input_path(repository, &verified.packet_path);
    let packet_bytes =
        bounded_file::read_regular(&packet_path, PACKET_LIMIT, "published review packet")?;
    require_exact_hash(
        &verified.packet_hash,
        &packet_bytes,
        "published review packet",
    )?;
    if manifest.packet.hash != verified.packet_hash {
        return Err("review manifest packet hash differs from the verified packet".to_owned());
    }

    authorize(
        &request,
        &authorization,
        classification.kind,
        classification.finding_resolution_request_index,
        packet_bytes.len(),
        manifest.packet.managed_payload_tokens,
    )?;
    let prior_delivery = receipt::capture_prior_delivery(repository, &request, &classification)?;

    for (path, expected, limit, label) in [
        (
            request_path,
            request_bytes.as_slice(),
            REQUEST_LIMIT,
            "delegated Slice review egress request",
        ),
        (
            authorization_path.as_path(),
            authorization_bytes.as_slice(),
            AUTHORIZATION_LIMIT,
            "delegated external review authorization",
        ),
    ] {
        if bounded_file::read_regular(path, limit, label)? != expected {
            return Err(format!("{label} changed during egress authorization"));
        }
    }
    if let Some(receipt) = &prior_delivery
        && bounded_file::read_regular(
            &receipt.path,
            DELIVERY_RECEIPT_LIMIT,
            "prior delegated delivery receipt",
        )? != receipt.bytes
    {
        return Err(
            "prior delegated delivery receipt changed during egress authorization".to_owned(),
        );
    }
    let current = verify(repository, &manifest_path, &request.manifest_hash)?;
    if current != verified {
        return Err("verified review chain changed during final revalidation".to_owned());
    }

    let request_id = digest(&request_bytes);
    let authorization_id = digest(&authorization_bytes);
    let review_kind = match classification.kind {
        ReviewKind::Original => "original",
        ReviewKind::FindingResolution => "finding_resolution",
    };
    let host = request.target.host().to_owned();
    let delivery = AuthorizedHostDelivery {
        request_id: request_id.clone(),
        authorization_id: authorization_id.clone(),
        authority: authorization.authority().to_owned(),
        review_kind,
        review_id: verified.review_id.clone(),
        candidate_commit: verified.candidate_commit.clone(),
        trusted_commit: verified.trusted_commit.clone(),
        packet_hash: verified.packet_hash.clone(),
        packet_bytes: packet_bytes.clone(),
        managed_payload_tokens: manifest.packet.managed_payload_tokens,
        host,
        execution_profile: request.execution_profile.clone(),
        fresh_session: matches!(request.session, Session::Fresh),
        session_id: match &request.session {
            Session::Fresh => None,
            Session::Resume { id } => Some(id.clone()),
        },
        prior_packet_hash: classification
            .prior
            .as_ref()
            .map(|prior| prior.packet_hash.clone()),
        prior_host_request_id: prior_delivery
            .as_ref()
            .map(|receipt| receipt.host_request_id.clone()),
        prior_execution_isolation: prior_delivery
            .as_ref()
            .and_then(|receipt| receipt.execution_isolation.clone()),
    };
    let document = DelegatedResultDocument {
        schema: DELEGATED_RESULT_SCHEMA,
        ok: true,
        status: "authorized",
        next_action: "deliver_delegated_once",
        request_id,
        authorization_id,
        authority: authorization.authority().to_owned(),
        review_kind: classification.kind,
        review_id: verified.review_id,
        candidate_commit: verified.candidate_commit,
        packet: PacketResult {
            path: verified.packet_path,
            hash: verified.packet_hash,
            bytes: packet_bytes.len(),
            managed_payload_tokens: manifest.packet.managed_payload_tokens,
        },
        target: request.target,
        execution_profile: request.execution_profile,
        session: request.session,
        limits: DelegatedDeliveryLimits {
            host_requests: 1,
            additional_host_requests: 0,
            retries: 0,
            steer: 0,
            fallback: 0,
            target_switch: false,
        },
    };
    Ok((document, delivery))
}

fn validate_request(request: &DelegatedRequest) -> Result<(), String> {
    if request.schema != DELEGATED_REQUEST_SCHEMA {
        return Err(format!(
            "unsupported delegated Slice review egress request schema `{}`; expected `{DELEGATED_REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.manifest_path, "manifest_path")?;
    require_sha256(&request.manifest_hash, "manifest_hash")?;
    require_sha256(&request.authorization_hash, "authorization_hash")?;
    target::validate_host(request.target.host())?;
    target::require_execution_profile(&request.execution_profile)?;
    if let Session::Resume { id } = &request.session {
        compact_token(id, MAX_SESSION_ID_BYTES, "resume session id")?;
    }
    if let Some(prior) = &request.prior_delivery {
        compact_path(&prior.path, "prior_delivery path")?;
        require_sha256(&prior.hash, "prior_delivery hash")?;
    }
    Ok(())
}
