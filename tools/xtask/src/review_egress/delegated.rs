use std::path::{Path, PathBuf};

use super::{
    AUTHORIZATION_LIMIT, DELIVERY_RECEIPT_LIMIT, MANIFEST_LIMIT, MAX_AUTHORIZED_TOKENS,
    MAX_SESSION_ID_BYTES, PACKET_LIMIT, REQUEST_LIMIT, ReviewClassification, VerifiedDeliveryRoute,
    bounded_file, classify_review_kind, compact_path, compact_token, digest, git,
    model::{
        AuthorizedDelegatedTarget, AuthorizedDelegatedTargetV1Alpha2,
        AuthorizedDelegatedTargetV1Alpha3, DELEGATED_AUTHORIZATION_SCHEMA,
        DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2, DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3,
        DELEGATED_DELIVERY_RECEIPT_SCHEMA, DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2,
        DELEGATED_EXECUTION_PROFILE, DELEGATED_REQUEST_SCHEMA, DELEGATED_RESULT_SCHEMA,
        DELEGATED_REVIEW_CHAIN_PROFILE, DelegatedAuthorizationDocument, DelegatedDeliveryLimits,
        DelegatedDeliveryReceipt, DelegatedRequest, DelegatedResultDocument, ManifestHeader,
        PacketResult, ReviewKind, Session,
    },
    require_exact_hash, require_sha256, resolve_input_path, review_delta,
};
use crate::review_packet::VerifiedReview;

const MAX_HOST_TARGETS: usize = 2;
const MAX_HOST_TOKEN_BYTES: usize = 32;
const MAX_HOST_REQUEST_ID_BYTES: usize = 256;
const MAX_FINDING_RESOLUTION_REQUESTS: usize = 63;

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

struct CapturedHostReceipt {
    path: PathBuf,
    bytes: Vec<u8>,
    host_request_id: String,
    execution_isolation: Option<String>,
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
    let prior_delivery = capture_prior_delivery(repository, &request, &classification)?;

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
    validate_host(request.target.host())?;
    require_execution_profile(&request.execution_profile)?;
    if let Session::Resume { id } = &request.session {
        compact_token(id, MAX_SESSION_ID_BYTES, "resume session id")?;
    }
    if let Some(prior) = &request.prior_delivery {
        compact_path(&prior.path, "prior_delivery path")?;
        require_sha256(&prior.hash, "prior_delivery hash")?;
    }
    Ok(())
}

fn canonical_authorization_path(repository: &Path) -> Result<PathBuf, String> {
    let common = git::trusted_output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common.trim());
    if common.file_name().and_then(|name| name.to_str()) != Some(".git") {
        return Err(
            "trusted Git common directory must be the repository `.git` directory".to_owned(),
        );
    }
    let root = common
        .parent()
        .ok_or_else(|| "trusted Git common directory has no repository parent".to_owned())?;
    Ok(root
        .join(".local-exclude")
        .join("authorizations")
        .join("external-review-delegated.json"))
}

fn validate_authorization(authorization: &DelegatedAuthorizationDocument) -> Result<(), String> {
    let (schema, status, target_count) = match authorization {
        DelegatedAuthorizationDocument::Alpha1(value) => {
            if value.schema != DELEGATED_AUTHORIZATION_SCHEMA {
                return Err(format!(
                    "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`, `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2}`, or `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3}`",
                    value.schema
                ));
            }
            (
                value.schema.as_str(),
                value.status.as_str(),
                value.targets.len(),
            )
        },
        DelegatedAuthorizationDocument::Alpha2(value) => {
            if value.schema != DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2 {
                return Err(format!(
                    "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`, `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2}`, or `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3}`",
                    value.schema
                ));
            }
            (
                value.schema.as_str(),
                value.status.as_str(),
                value.targets.len(),
            )
        },
        DelegatedAuthorizationDocument::Alpha3(value) => {
            if value.schema != DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3 {
                return Err(format!(
                    "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`, `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2}`, or `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3}`",
                    value.schema
                ));
            }
            if value.review_chain_profile != DELEGATED_REVIEW_CHAIN_PROFILE {
                return Err(format!(
                    "delegated authorization v1alpha3 requires review_chain_profile `{DELEGATED_REVIEW_CHAIN_PROFILE}`"
                ));
            }
            (
                value.schema.as_str(),
                value.status.as_str(),
                value.targets.len(),
            )
        },
    };
    if status != "active" {
        return Err("delegated external review authorization is not active".to_owned());
    }
    let Some(owner) = authorization.authority().strip_prefix("human/") else {
        return Err("delegated external review authority must start with `human/`".to_owned());
    };
    compact_token(owner, 122, "authorization human owner")?;
    compact_token(authorization.authority(), 128, "authorization authority")?;
    if target_count == 0 || target_count > MAX_HOST_TARGETS {
        return Err(format!(
            "delegated external review authorization requires 1..={MAX_HOST_TARGETS} targets"
        ));
    }
    let mut hosts = std::collections::BTreeSet::new();
    match authorization {
        DelegatedAuthorizationDocument::Alpha1(value) => {
            for target in &value.targets {
                validate_target_limits(
                    &mut hosts,
                    &target.host,
                    &target.execution_profile,
                    target.max_packet_bytes,
                    target.max_managed_payload_tokens,
                )?;
                if !target.allow_original_fresh && !target.allow_finding_resolution_resume {
                    return Err(
                        "an authorized delegated target must allow at least one review request kind"
                            .to_owned(),
                    );
                }
            }
        },
        DelegatedAuthorizationDocument::Alpha2(value) => {
            for target in &value.targets {
                validate_target_limits(
                    &mut hosts,
                    &target.host,
                    &target.execution_profile,
                    target.max_packet_bytes,
                    target.max_managed_payload_tokens,
                )?;
                if target.max_original_fresh_requests > 1
                    || target.max_finding_resolution_resume_requests > 1
                    || target.max_total_requests
                        != target.max_original_fresh_requests
                            + target.max_finding_resolution_resume_requests
                    || target.max_total_requests == 0
                {
                    return Err(
                        "delegated authorization v1alpha2 requires explicit 0..=1 original and finding-resolution limits whose nonzero sum equals max_total_requests"
                            .to_owned(),
                    );
                }
            }
        },
        DelegatedAuthorizationDocument::Alpha3(value) => {
            for target in &value.targets {
                validate_target_limits(
                    &mut hosts,
                    &target.host,
                    &target.execution_profile,
                    target.max_packet_bytes,
                    target.max_managed_payload_tokens,
                )?;
                if target.max_original_fresh_requests > 1
                    || target.max_finding_resolution_resume_requests
                        > MAX_FINDING_RESOLUTION_REQUESTS
                    || target.max_total_requests
                        != target.max_original_fresh_requests
                            + target.max_finding_resolution_resume_requests
                    || target.max_total_requests == 0
                {
                    return Err(format!(
                        "delegated authorization v1alpha3 requires original 0..=1 and finding-resolution 0..={MAX_FINDING_RESOLUTION_REQUESTS} limits whose nonzero sum equals max_total_requests"
                    ));
                }
            }
        },
    }
    debug_assert!(matches!(
        schema,
        DELEGATED_AUTHORIZATION_SCHEMA
            | DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2
            | DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3
    ));
    Ok(())
}

fn validate_target_limits<'a>(
    hosts: &mut std::collections::BTreeSet<&'a str>,
    host: &'a str,
    execution_profile: &str,
    max_packet_bytes: usize,
    max_managed_payload_tokens: usize,
) -> Result<(), String> {
    validate_host(host)?;
    require_execution_profile(execution_profile)?;
    if !hosts.insert(host) {
        return Err("delegated external review targets must be unique".to_owned());
    }
    if max_packet_bytes == 0 || max_packet_bytes > PACKET_LIMIT {
        return Err(format!(
            "authorized max_packet_bytes must be within 1..={PACKET_LIMIT}"
        ));
    }
    if max_managed_payload_tokens == 0 || max_managed_payload_tokens > MAX_AUTHORIZED_TOKENS {
        return Err(format!(
            "authorized max_managed_payload_tokens must be within 1..={MAX_AUTHORIZED_TOKENS}"
        ));
    }
    Ok(())
}

fn authorize(
    request: &DelegatedRequest,
    authorization: &DelegatedAuthorizationDocument,
    review_kind: ReviewKind,
    finding_resolution_request_index: usize,
    packet_bytes: usize,
    managed_payload_tokens: usize,
) -> Result<(), String> {
    let target = authorized_target(authorization, request)?;
    match (review_kind, &request.session) {
        (ReviewKind::Original, Session::Fresh) if target.allow_original_fresh() => {},
        (ReviewKind::FindingResolution, Session::Resume { .. })
            if finding_resolution_request_index
                <= target.max_finding_resolution_resume_requests() => {},
        (ReviewKind::Original, Session::Fresh) => {
            return Err("the target does not authorize an original fresh review".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Resume { .. }) => {
            return Err("the target does not authorize a finding-resolution resume".to_owned());
        },
        (ReviewKind::Original, Session::Resume { .. }) => {
            return Err("an original review requires a fresh Session".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Fresh) => {
            return Err(
                "a finding-resolution review requires the existing reviewer Session".to_owned(),
            );
        },
    }
    if packet_bytes > target.max_packet_bytes() {
        return Err(format!(
            "review packet has {packet_bytes} bytes, exceeding the authorized {}-byte target limit",
            target.max_packet_bytes()
        ));
    }
    if managed_payload_tokens > target.max_managed_payload_tokens() {
        return Err(format!(
            "review packet has {managed_payload_tokens} managed tokens, exceeding the authorized {}-token target limit",
            target.max_managed_payload_tokens()
        ));
    }
    Ok(())
}

enum AuthorizedTarget<'a> {
    Alpha1(&'a AuthorizedDelegatedTarget),
    Alpha2(&'a AuthorizedDelegatedTargetV1Alpha2),
    Alpha3(&'a AuthorizedDelegatedTargetV1Alpha3),
}

impl AuthorizedTarget<'_> {
    fn allow_original_fresh(&self) -> bool {
        match self {
            Self::Alpha1(value) => value.allow_original_fresh,
            Self::Alpha2(value) => value.max_original_fresh_requests == 1,
            Self::Alpha3(value) => value.max_original_fresh_requests == 1,
        }
    }

    fn max_finding_resolution_resume_requests(&self) -> usize {
        match self {
            Self::Alpha1(value) => usize::from(value.allow_finding_resolution_resume),
            Self::Alpha2(value) => value.max_finding_resolution_resume_requests,
            Self::Alpha3(value) => value.max_finding_resolution_resume_requests,
        }
    }

    fn max_packet_bytes(&self) -> usize {
        match self {
            Self::Alpha1(value) => value.max_packet_bytes,
            Self::Alpha2(value) => value.max_packet_bytes,
            Self::Alpha3(value) => value.max_packet_bytes,
        }
    }

    fn max_managed_payload_tokens(&self) -> usize {
        match self {
            Self::Alpha1(value) => value.max_managed_payload_tokens,
            Self::Alpha2(value) => value.max_managed_payload_tokens,
            Self::Alpha3(value) => value.max_managed_payload_tokens,
        }
    }
}

fn authorized_target<'a>(
    authorization: &'a DelegatedAuthorizationDocument,
    request: &DelegatedRequest,
) -> Result<AuthorizedTarget<'a>, String> {
    match authorization {
        DelegatedAuthorizationDocument::Alpha1(value) => value
            .targets
            .iter()
            .find(|target| {
                target.host == request.target.host()
                    && target.execution_profile == request.execution_profile
            })
            .map(AuthorizedTarget::Alpha1),
        DelegatedAuthorizationDocument::Alpha2(value) => value
            .targets
            .iter()
            .find(|target| {
                target.host == request.target.host()
                    && target.execution_profile == request.execution_profile
            })
            .map(AuthorizedTarget::Alpha2),
        DelegatedAuthorizationDocument::Alpha3(value) => value
            .targets
            .iter()
            .find(|target| {
                target.host == request.target.host()
                    && target.execution_profile == request.execution_profile
            })
            .map(AuthorizedTarget::Alpha3),
    }
    .ok_or_else(|| "requested delegated review target is not authorized".to_owned())
}

fn capture_prior_delivery(
    repository: &Path,
    request: &DelegatedRequest,
    classification: &ReviewClassification,
) -> Result<Option<CapturedHostReceipt>, String> {
    let Some(prior) = &classification.prior else {
        if request.prior_delivery.is_some() {
            return Err("an original review must not name prior_delivery evidence".to_owned());
        }
        return Ok(None);
    };
    let reference = request
        .prior_delivery
        .as_ref()
        .ok_or_else(|| "a finding-resolution review requires prior_delivery evidence".to_owned())?;
    let path = resolve_input_path(repository, &reference.path);
    let bytes = bounded_file::read_regular(
        &path,
        DELIVERY_RECEIPT_LIMIT,
        "prior delegated delivery receipt",
    )?;
    require_exact_hash(&reference.hash, &bytes, "prior delegated delivery receipt")?;
    let receipt = parse_delivery_receipt(&bytes, "prior delegated delivery receipt")?;
    if receipt.review_id != prior.review_id || receipt.packet_hash != prior.packet_hash {
        return Err("prior delegated receipt does not match the original review packet".to_owned());
    }
    if receipt.target != request.target || receipt.execution_profile != request.execution_profile {
        return Err(
            "finding-resolution delegated target differs from the original delivery target"
                .to_owned(),
        );
    }
    let Session::Resume { id } = &request.session else {
        return Err(
            "a finding-resolution review requires the existing reviewer Session".to_owned(),
        );
    };
    if receipt.session_id != *id {
        return Err(
            "finding-resolution Session differs from the original delivery Session".to_owned(),
        );
    }
    Ok(Some(CapturedHostReceipt {
        path,
        bytes,
        host_request_id: receipt.host_request_id,
        execution_isolation: receipt.execution_isolation,
    }))
}

pub(super) fn verify_completed_delivery_bytes(
    bytes: &[u8],
    review: &VerifiedReview,
) -> Result<VerifiedDeliveryRoute, String> {
    let receipt = parse_delivery_receipt(bytes, "delegated external review delivery receipt")?;
    if receipt.review_id != review.review_id || receipt.packet_hash != review.packet_hash {
        return Err(
            "delegated external review delivery receipt does not match the reviewed packet"
                .to_owned(),
        );
    }
    Ok(VerifiedDeliveryRoute::Delegated {
        host: receipt.target.host().to_owned(),
        session_id: receipt.session_id,
    })
}

fn parse_delivery_receipt(bytes: &[u8], label: &str) -> Result<DelegatedDeliveryReceipt, String> {
    let receipt: DelegatedDeliveryReceipt =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid {label}: {error}"))?;
    if !matches!(
        receipt.schema.as_str(),
        DELEGATED_DELIVERY_RECEIPT_SCHEMA | DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2
    ) {
        return Err(format!(
            "unsupported delegated delivery receipt schema `{}`; expected `{DELEGATED_DELIVERY_RECEIPT_SCHEMA}` or `{DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2}`",
            receipt.schema
        ));
    }
    require_sha256(&receipt.review_id, "delivery ReviewId")?;
    require_sha256(&receipt.packet_hash, "delivery packet hash")?;
    validate_host(receipt.target.host())?;
    require_execution_profile(&receipt.execution_profile)?;
    match (
        receipt.schema.as_str(),
        receipt.execution_isolation.as_deref(),
    ) {
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA, None) => {},
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2, Some(isolation))
            if receipt.target.host() == "grok"
                && matches!(
                    isolation,
                    crate::grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE
                        | crate::grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE
                ) => {},
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA, Some(_)) => {
            return Err(
                "delegated delivery receipt v1alpha1 must not name execution isolation".to_owned(),
            );
        },
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2, _) => {
            return Err(
                "delegated delivery receipt v1alpha2 requires an exact Grok execution isolation"
                    .to_owned(),
            );
        },
        _ => unreachable!("validated delegated receipt schema"),
    }
    compact_token(
        &receipt.session_id,
        MAX_SESSION_ID_BYTES,
        "delivery session id",
    )?;
    compact_token(
        &receipt.host_request_id,
        MAX_HOST_REQUEST_ID_BYTES,
        "delivery host request id",
    )?;
    if receipt.host_request_count != 1 {
        return Err("delegated delivery receipt must record exactly one host request".to_owned());
    }
    Ok(receipt)
}

fn validate_host(host: &str) -> Result<(), String> {
    compact_token(host, MAX_HOST_TOKEN_BYTES, "delegated host")?;
    if matches!(host, "codex" | "grok") {
        Ok(())
    } else {
        Err(format!(
            "unsupported delegated review host `{host}`; expected `codex` or `grok`"
        ))
    }
}

fn require_execution_profile(profile: &str) -> Result<(), String> {
    if profile == DELEGATED_EXECUTION_PROFILE {
        Ok(())
    } else {
        Err(format!(
            "delegated review requires execution profile `{DELEGATED_EXECUTION_PROFILE}`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_egress::model::{
        AuthorizedDelegatedTarget, DelegatedAuthorization, DelegatedAuthorizationV1Alpha2,
        DelegatedAuthorizationV1Alpha3,
    };

    fn target(host: &str) -> AuthorizedDelegatedTarget {
        AuthorizedDelegatedTarget {
            host: host.to_owned(),
            execution_profile: DELEGATED_EXECUTION_PROFILE.to_owned(),
            max_packet_bytes: 4_000_000,
            max_managed_payload_tokens: 500_000,
            allow_original_fresh: true,
            allow_finding_resolution_resume: true,
        }
    }

    // 지원 host가 두 개뿐인 현재 계약은 승인 후보 수도 exact 2로 닫아 임의 이름이
    // standing authority의 후보 공간을 넓히지 못하게 합니다.
    #[test]
    fn authorization_is_bounded_to_the_two_supported_hosts() {
        let mut authorization = DelegatedAuthorization {
            schema: DELEGATED_AUTHORIZATION_SCHEMA.to_owned(),
            authority: "human/yon".to_owned(),
            status: "active".to_owned(),
            targets: vec![target("codex"), target("grok")],
        };
        validate_authorization(&DelegatedAuthorizationDocument::Alpha1(authorization)).unwrap();
        authorization = DelegatedAuthorization {
            schema: DELEGATED_AUTHORIZATION_SCHEMA.to_owned(),
            authority: "human/yon".to_owned(),
            status: "active".to_owned(),
            targets: vec![target("codex"), target("grok")],
        };
        authorization.targets.push(target("codex"));
        assert!(
            validate_authorization(&DelegatedAuthorizationDocument::Alpha1(authorization))
                .unwrap_err()
                .contains("1..=2")
        );
    }

    // 새 승인은 `허용` boolean 대신 original 1회와 resolution 1회의 합계를 exact하게
    // 기록해 사람이 말한 총 요청 수와 실행기의 round 제한이 어긋나지 않게 합니다.
    #[test]
    fn alpha2_authorization_requires_consistent_round_limits() {
        let target = AuthorizedDelegatedTargetV1Alpha2 {
            host: "codex".to_owned(),
            execution_profile: DELEGATED_EXECUTION_PROFILE.to_owned(),
            max_packet_bytes: 4_000_000,
            max_managed_payload_tokens: 500_000,
            max_original_fresh_requests: 1,
            max_finding_resolution_resume_requests: 1,
            max_total_requests: 2,
        };
        validate_authorization(&DelegatedAuthorizationDocument::Alpha2(
            DelegatedAuthorizationV1Alpha2 {
                schema: DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2.to_owned(),
                authority: "human/yon".to_owned(),
                status: "active".to_owned(),
                targets: vec![target],
            },
        ))
        .unwrap();

        let inconsistent = AuthorizedDelegatedTargetV1Alpha2 {
            host: "codex".to_owned(),
            execution_profile: DELEGATED_EXECUTION_PROFILE.to_owned(),
            max_packet_bytes: 4_000_000,
            max_managed_payload_tokens: 500_000,
            max_original_fresh_requests: 1,
            max_finding_resolution_resume_requests: 1,
            max_total_requests: 1,
        };
        assert!(
            validate_authorization(&DelegatedAuthorizationDocument::Alpha2(
                DelegatedAuthorizationV1Alpha2 {
                    schema: DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2.to_owned(),
                    authority: "human/yon".to_owned(),
                    status: "active".to_owned(),
                    targets: vec![inconsistent],
                },
            ))
            .unwrap_err()
            .contains("sum equals")
        );
    }

    // v1alpha3은 recursive review chain이 계산한 resolution request index를 사람의
    // explicit 총량과 비교해 두 번째 후속은 허용하고 그 다음 요청은 fail closed합니다.
    #[test]
    fn alpha3_authorization_bounds_recursive_resolution_requests() {
        let authorization_text = serde_json::json!({
            "schema": DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3,
            "authority": "human/yon",
            "status": "active",
            "review_chain_profile": DELEGATED_REVIEW_CHAIN_PROFILE,
            "targets": [{
                "host": "codex",
                "execution_profile": DELEGATED_EXECUTION_PROFILE,
                "max_packet_bytes": 4_000_000,
                "max_managed_payload_tokens": 500_000,
                "max_original_fresh_requests": 1,
                "max_finding_resolution_resume_requests": 2,
                "max_total_requests": 3
            }]
        });
        let authorization: DelegatedAuthorizationDocument =
            serde_json::from_value(authorization_text).unwrap();
        assert!(matches!(
            authorization,
            DelegatedAuthorizationDocument::Alpha3(_)
        ));
        validate_authorization(&authorization).unwrap();

        let typed_authorization =
            DelegatedAuthorizationDocument::Alpha3(DelegatedAuthorizationV1Alpha3 {
                schema: DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3.to_owned(),
                authority: "human/yon".to_owned(),
                status: "active".to_owned(),
                review_chain_profile: DELEGATED_REVIEW_CHAIN_PROFILE.to_owned(),
                targets: vec![AuthorizedDelegatedTargetV1Alpha3 {
                    host: "codex".to_owned(),
                    execution_profile: DELEGATED_EXECUTION_PROFILE.to_owned(),
                    max_packet_bytes: 4_000_000,
                    max_managed_payload_tokens: 500_000,
                    max_original_fresh_requests: 1,
                    max_finding_resolution_resume_requests: 2,
                    max_total_requests: 3,
                }],
            });
        validate_authorization(&typed_authorization).unwrap();
        let request = DelegatedRequest {
            schema: DELEGATED_REQUEST_SCHEMA.to_owned(),
            manifest_path: "manifest.json".to_owned(),
            manifest_hash: format!("sha256:{}", "1".repeat(64)),
            authorization_hash: format!("sha256:{}", "2".repeat(64)),
            target: crate::review_egress::model::DelegatedTarget::DelegatedHost {
                host: "codex".to_owned(),
            },
            execution_profile: DELEGATED_EXECUTION_PROFILE.to_owned(),
            session: Session::Resume {
                id: "session".to_owned(),
            },
            prior_delivery: None,
        };

        authorize(
            &request,
            &authorization,
            ReviewKind::FindingResolution,
            2,
            1,
            1,
        )
        .unwrap();
        assert!(
            authorize(
                &request,
                &authorization,
                ReviewKind::FindingResolution,
                3,
                1,
                1,
            )
            .unwrap_err()
            .contains("does not authorize")
        );
    }

    // delegated receipt는 host identity와 profile만 허용하고 managed Provider 필드를
    // 끼워 넣어 두 identity 공간을 합치는 artifact를 거부합니다.
    #[test]
    fn delegated_receipt_rejects_fabricated_provider_fields() {
        let receipt = serde_json::json!({
            "schema": DELEGATED_DELIVERY_RECEIPT_SCHEMA,
            "review_id": format!("sha256:{}", "1".repeat(64)),
            "packet_hash": format!("sha256:{}", "2".repeat(64)),
            "target": {"kind": "delegated_host", "host": "codex"},
            "execution_profile": DELEGATED_EXECUTION_PROFILE,
            "session_id": "session-a",
            "host_request_id": "request-a",
            "host_request_count": 1
        });
        parse_delivery_receipt(&serde_json::to_vec(&receipt).unwrap(), "receipt").unwrap();

        let mut fabricated = receipt;
        fabricated["provider_request_id"] = "provider-request-a".into();
        assert!(
            parse_delivery_receipt(&serde_json::to_vec(&fabricated).unwrap(), "receipt")
                .unwrap_err()
                .contains("unknown field")
        );
    }

    // v1alpha2 receipt는 Grok 요청에 실제로 선택된 native 또는 Yo outer isolation을
    // 반드시 기록하고, 기존 v1alpha1이나 Codex receipt로 그 의미를 위조하지 못합니다.
    #[test]
    fn delegated_receipt_v1alpha2_binds_exact_grok_isolation() {
        let receipt = serde_json::json!({
            "schema": DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2,
            "review_id": format!("sha256:{}", "1".repeat(64)),
            "packet_hash": format!("sha256:{}", "2".repeat(64)),
            "target": {"kind": "delegated_host", "host": "grok"},
            "execution_profile": DELEGATED_EXECUTION_PROFILE,
            "execution_isolation": crate::grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE,
            "session_id": "session-a",
            "host_request_id": "request-a",
            "host_request_count": 1
        });
        parse_delivery_receipt(&serde_json::to_vec(&receipt).unwrap(), "receipt").unwrap();

        let mut missing = receipt.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("execution_isolation");
        assert!(
            parse_delivery_receipt(&serde_json::to_vec(&missing).unwrap(), "receipt")
                .unwrap_err()
                .contains("requires an exact Grok execution isolation")
        );

        let mut legacy = receipt.clone();
        legacy["schema"] = DELEGATED_DELIVERY_RECEIPT_SCHEMA.into();
        assert!(
            parse_delivery_receipt(&serde_json::to_vec(&legacy).unwrap(), "receipt")
                .unwrap_err()
                .contains("must not name execution isolation")
        );

        let mut codex = receipt;
        codex["target"]["host"] = "codex".into();
        assert!(
            parse_delivery_receipt(&serde_json::to_vec(&codex).unwrap(), "receipt")
                .unwrap_err()
                .contains("requires an exact Grok execution isolation")
        );
    }

    // host-owned read tools를 managed no-tools 증거로 오인하지 않도록 delegated limit
    // artifact에는 tool_execution 필드 자체가 존재하지 않습니다.
    #[test]
    fn delegated_limits_do_not_publish_a_managed_tool_claim() {
        let value = serde_json::to_value(DelegatedDeliveryLimits {
            host_requests: 1,
            additional_host_requests: 0,
            retries: 0,
            steer: 0,
            fallback: 0,
            target_switch: false,
        })
        .unwrap();
        assert!(value.get("tool_execution").is_none());
        assert_eq!(value["host_requests"], 1);
    }
}
