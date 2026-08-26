use std::path::{Path, PathBuf};

use super::{
    AUTHORIZATION_LIMIT, DELIVERY_RECEIPT_LIMIT, MANIFEST_LIMIT, MAX_AUTHORIZED_TOKENS,
    MAX_SESSION_ID_BYTES, PACKET_LIMIT, REQUEST_LIMIT, ReviewClassification, VerifiedDeliveryRoute,
    bounded_file, classify_review_kind, compact_path, compact_token, digest, git,
    model::{
        DELEGATED_AUTHORIZATION_SCHEMA, DELEGATED_DELIVERY_RECEIPT_SCHEMA,
        DELEGATED_EXECUTION_PROFILE, DELEGATED_REQUEST_SCHEMA, DELEGATED_RESULT_SCHEMA,
        DelegatedAuthorization, DelegatedDeliveryLimits, DelegatedDeliveryReceipt,
        DelegatedRequest, DelegatedResultDocument, ManifestHeader, PacketResult, ReviewKind,
        Session,
    },
    require_exact_hash, require_sha256, resolve_input_path, review_delta,
};
use crate::review_packet::VerifiedReview;

const MAX_HOST_TARGETS: usize = 2;
const MAX_HOST_TOKEN_BYTES: usize = 32;
const MAX_HOST_REQUEST_ID_BYTES: usize = 256;

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
}

struct CapturedHostReceipt {
    path: PathBuf,
    bytes: Vec<u8>,
    host_request_id: String,
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
    let authorization: DelegatedAuthorization = serde_json::from_slice(&authorization_bytes)
        .map_err(|error| {
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
        authority: authorization.authority.clone(),
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
    };
    let document = DelegatedResultDocument {
        schema: DELEGATED_RESULT_SCHEMA,
        ok: true,
        status: "authorized",
        next_action: "deliver_delegated_once",
        request_id,
        authorization_id,
        authority: authorization.authority,
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

fn validate_authorization(authorization: &DelegatedAuthorization) -> Result<(), String> {
    if authorization.schema != DELEGATED_AUTHORIZATION_SCHEMA {
        return Err(format!(
            "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`",
            authorization.schema
        ));
    }
    if authorization.status != "active" {
        return Err("delegated external review authorization is not active".to_owned());
    }
    let Some(owner) = authorization.authority.strip_prefix("human/") else {
        return Err("delegated external review authority must start with `human/`".to_owned());
    };
    compact_token(owner, 122, "authorization human owner")?;
    compact_token(&authorization.authority, 128, "authorization authority")?;
    if authorization.targets.is_empty() || authorization.targets.len() > MAX_HOST_TARGETS {
        return Err(format!(
            "delegated external review authorization requires 1..={MAX_HOST_TARGETS} targets"
        ));
    }
    let mut hosts = std::collections::BTreeSet::new();
    for target in &authorization.targets {
        validate_host(&target.host)?;
        require_execution_profile(&target.execution_profile)?;
        if !hosts.insert(target.host.as_str()) {
            return Err("delegated external review targets must be unique".to_owned());
        }
        if target.max_packet_bytes == 0 || target.max_packet_bytes > PACKET_LIMIT {
            return Err(format!(
                "authorized max_packet_bytes must be within 1..={PACKET_LIMIT}"
            ));
        }
        if target.max_managed_payload_tokens == 0
            || target.max_managed_payload_tokens > MAX_AUTHORIZED_TOKENS
        {
            return Err(format!(
                "authorized max_managed_payload_tokens must be within 1..={MAX_AUTHORIZED_TOKENS}"
            ));
        }
        if !target.allow_original_fresh && !target.allow_finding_resolution_resume {
            return Err(
                "an authorized delegated target must allow at least one review request kind"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn authorize(
    request: &DelegatedRequest,
    authorization: &DelegatedAuthorization,
    review_kind: ReviewKind,
    packet_bytes: usize,
    managed_payload_tokens: usize,
) -> Result<(), String> {
    let target = authorization
        .targets
        .iter()
        .find(|target| {
            target.host == request.target.host()
                && target.execution_profile == request.execution_profile
        })
        .ok_or_else(|| "requested delegated review target is not authorized".to_owned())?;
    match (review_kind, &request.session) {
        (ReviewKind::Original, Session::Fresh) if target.allow_original_fresh => {},
        (ReviewKind::FindingResolution, Session::Resume { .. })
            if target.allow_finding_resolution_resume => {},
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
    if packet_bytes > target.max_packet_bytes {
        return Err(format!(
            "review packet has {packet_bytes} bytes, exceeding the authorized {}-byte target limit",
            target.max_packet_bytes
        ));
    }
    if managed_payload_tokens > target.max_managed_payload_tokens {
        return Err(format!(
            "review packet has {managed_payload_tokens} managed tokens, exceeding the authorized {}-token target limit",
            target.max_managed_payload_tokens
        ));
    }
    Ok(())
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
    if receipt.schema != DELEGATED_DELIVERY_RECEIPT_SCHEMA {
        return Err(format!(
            "unsupported delegated delivery receipt schema `{}`; expected `{DELEGATED_DELIVERY_RECEIPT_SCHEMA}`",
            receipt.schema
        ));
    }
    require_sha256(&receipt.review_id, "delivery ReviewId")?;
    require_sha256(&receipt.packet_hash, "delivery packet hash")?;
    validate_host(receipt.target.host())?;
    require_execution_profile(&receipt.execution_profile)?;
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
    use crate::review_egress::model::AuthorizedDelegatedTarget;

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
        validate_authorization(&authorization).unwrap();
        authorization.targets.push(target("codex"));
        assert!(
            validate_authorization(&authorization)
                .unwrap_err()
                .contains("1..=2")
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
