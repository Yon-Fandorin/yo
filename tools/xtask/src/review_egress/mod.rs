mod model;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use model::{
    AUTHORIZATION_SCHEMA, Authorization, DELIVERY_RECEIPT_SCHEMA, DeliveryLimits, DeliveryReceipt,
    ManifestHeader, PacketResult, REQUEST_SCHEMA, RESULT_SCHEMA, Request, ResultDocument,
    ReviewKind, Route, Session,
};

use crate::{
    bounded_file, git, review_delta,
    review_packet::{self, VerifiedReview},
    review_protocol::{digest, resolve_input_path},
};

const REQUEST_LIMIT: usize = 64 * 1024;
const AUTHORIZATION_LIMIT: usize = 64 * 1024;
const DELIVERY_RECEIPT_LIMIT: usize = 64 * 1024;
const MANIFEST_LIMIT: usize = 8 * 1024 * 1024;
const PACKET_LIMIT: usize = 32 * 1024 * 1024;
const MAX_ROUTES: usize = 16;
const MAX_ROUTE_TOKEN_BYTES: usize = 128;
const MAX_SESSION_ID_BYTES: usize = 256;
const MAX_AUTHORIZED_TOKENS: usize = 1_000_000;

#[derive(Debug)]
struct ReviewClassification {
    kind: ReviewKind,
    prior: Option<PriorReview>,
}

#[derive(Debug)]
struct PriorReview {
    review_id: String,
    packet_hash: String,
}

#[derive(Debug)]
struct CapturedDeliveryReceipt {
    path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedDeliveryRoute {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedDelivery {
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
    pub(crate) provider: String,
    pub(crate) account: String,
    pub(crate) model: String,
    pub(crate) fresh_session: bool,
}

struct FinalRevalidation<'a> {
    request_path: &'a Path,
    request_bytes: &'a [u8],
    authorization_path: &'a Path,
    authorization_bytes: &'a [u8],
    manifest_path: &'a Path,
    manifest_hash: &'a str,
    expected_review: &'a VerifiedReview,
    prior_delivery: Option<&'a CapturedDeliveryReceipt>,
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let output = evaluate(repository, request_path)?;
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| format!("cannot encode Slice review egress result: {error}"))?
    );
    Ok(())
}

fn evaluate(repository: &Path, request_path: &Path) -> Result<ResultDocument, String> {
    evaluate_with(repository, request_path, &|repository, manifest, hash| {
        review_delta::verify_chain_head(repository, manifest, hash, &mut BTreeSet::new(), 0)
    })
}

pub(crate) fn authorize_delivery(
    repository: &Path,
    request_path: &Path,
) -> Result<AuthorizedDelivery, String> {
    authorize_with(repository, request_path, &|repository, manifest, hash| {
        review_delta::verify_chain_head(repository, manifest, hash, &mut BTreeSet::new(), 0)
    })
    .map(|(_, delivery)| delivery)
}

fn evaluate_with(
    repository: &Path,
    request_path: &Path,
    verify: &dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String>,
) -> Result<ResultDocument, String> {
    authorize_with(repository, request_path, verify).map(|(document, _)| document)
}

fn authorize_with(
    repository: &Path,
    request_path: &Path,
    verify: &dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String>,
) -> Result<(ResultDocument, AuthorizedDelivery), String> {
    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice review egress request")?;
    let request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice review egress request {}: {error}",
            request_path.display()
        )
    })?;
    validate_request(&request)?;

    let authorization_path = canonical_authorization_path(repository)?;
    let authorization_bytes = bounded_file::read_regular(
        &authorization_path,
        AUTHORIZATION_LIMIT,
        "external review standing authorization",
    )?;
    require_exact_hash(
        &request.authorization_hash,
        &authorization_bytes,
        "external review standing authorization",
    )?;
    let authorization: Authorization =
        serde_json::from_slice(&authorization_bytes).map_err(|error| {
            format!(
                "invalid external review standing authorization {}: {error}",
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

    final_revalidate(
        repository,
        &FinalRevalidation {
            request_path,
            request_bytes: &request_bytes,
            authorization_path: &authorization_path,
            authorization_bytes: &authorization_bytes,
            manifest_path: &manifest_path,
            manifest_hash: &request.manifest_hash,
            expected_review: &verified,
            prior_delivery: prior_delivery.as_ref(),
        },
        verify,
    )?;

    let request_id = digest(&request_bytes);
    let authorization_id = digest(&authorization_bytes);
    let review_kind = match classification.kind {
        ReviewKind::Original => "original",
        ReviewKind::FindingResolution => "finding_resolution",
    };
    let delivery = AuthorizedDelivery {
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
        provider: request.route.provider.clone(),
        account: request.route.account.clone(),
        model: request.route.model.clone(),
        fresh_session: matches!(request.session, Session::Fresh),
    };
    let document = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        status: "authorized",
        next_action: "deliver_once",
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
        route: request.route,
        session: request.session,
        limits: DeliveryLimits {
            provider_requests: 1,
            additional_provider_requests: 0,
            retries: 0,
            steer: 0,
            fallback: 0,
            second_provider: false,
            tool_execution: false,
        },
    };
    Ok((document, delivery))
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice review egress request schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.manifest_path, "manifest_path")?;
    require_sha256(&request.manifest_hash, "manifest_hash")?;
    require_sha256(&request.authorization_hash, "authorization_hash")?;
    validate_route(&request.route)?;
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
        .join("external-review.json"))
}

fn validate_authorization(authorization: &Authorization) -> Result<(), String> {
    if authorization.schema != AUTHORIZATION_SCHEMA {
        return Err(format!(
            "unsupported external review standing authorization schema `{}`; expected `{AUTHORIZATION_SCHEMA}`",
            authorization.schema
        ));
    }
    if authorization.status != "active" {
        return Err("external review standing authorization is not active".to_owned());
    }
    let Some(owner) = authorization.authority.strip_prefix("human/") else {
        return Err(
            "external review standing authorization authority must start with `human/`".to_owned(),
        );
    };
    compact_token(owner, 122, "authorization human owner")?;
    compact_token(&authorization.authority, 128, "authorization authority")?;
    if authorization.routes.is_empty() || authorization.routes.len() > MAX_ROUTES {
        return Err(format!(
            "external review standing authorization requires 1..={MAX_ROUTES} routes"
        ));
    }

    let mut routes = BTreeSet::new();
    for route in &authorization.routes {
        let identity = Route {
            provider: route.provider.clone(),
            account: route.account.clone(),
            model: route.model.clone(),
        };
        validate_route(&identity)?;
        if !routes.insert((
            route.provider.as_str(),
            route.account.as_str(),
            route.model.as_str(),
        )) {
            return Err("external review standing authorization routes must be unique".to_owned());
        }
        if route.max_packet_bytes == 0 || route.max_packet_bytes > PACKET_LIMIT {
            return Err(format!(
                "authorized max_packet_bytes must be within 1..={PACKET_LIMIT}"
            ));
        }
        if route.max_managed_payload_tokens == 0
            || route.max_managed_payload_tokens > MAX_AUTHORIZED_TOKENS
        {
            return Err(format!(
                "authorized max_managed_payload_tokens must be within 1..={MAX_AUTHORIZED_TOKENS}"
            ));
        }
        if !route.allow_original_fresh && !route.allow_finding_resolution_resume {
            return Err(
                "an authorized route must allow at least one review request kind".to_owned(),
            );
        }
    }
    Ok(())
}

fn authorize(
    request: &Request,
    authorization: &Authorization,
    review_kind: ReviewKind,
    packet_bytes: usize,
    managed_payload_tokens: usize,
) -> Result<(), String> {
    let route = authorization
        .routes
        .iter()
        .find(|authorized| {
            authorized.provider == request.route.provider
                && authorized.account == request.route.account
                && authorized.model == request.route.model
        })
        .ok_or_else(|| "requested external review route is not authorized".to_owned())?;

    match (review_kind, &request.session) {
        (ReviewKind::Original, Session::Fresh) if route.allow_original_fresh => {},
        (ReviewKind::FindingResolution, Session::Resume { .. })
            if route.allow_finding_resolution_resume => {},
        (ReviewKind::Original, Session::Fresh) => {
            return Err("the route does not authorize an original fresh review".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Resume { .. }) => {
            return Err("the route does not authorize a finding-resolution resume".to_owned());
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

    if packet_bytes > route.max_packet_bytes {
        return Err(format!(
            "review packet has {packet_bytes} bytes, exceeding the authorized {}-byte route limit",
            route.max_packet_bytes
        ));
    }
    if managed_payload_tokens > route.max_managed_payload_tokens {
        return Err(format!(
            "review packet has {managed_payload_tokens} managed tokens, exceeding the authorized {}-token route limit",
            route.max_managed_payload_tokens
        ));
    }
    Ok(())
}

fn classify_review_kind(
    repository: &Path,
    manifest: &ManifestHeader,
) -> Result<ReviewClassification, String> {
    if review_packet::is_original_manifest_schema(&manifest.schema) {
        return Ok(ReviewClassification {
            kind: ReviewKind::Original,
            prior: None,
        });
    }

    let prior = manifest
        .inputs
        .as_ref()
        .and_then(|inputs| inputs.prior_manifest.as_ref())
        .ok_or_else(|| "finding-resolution manifest has no prior_manifest".to_owned())?;
    require_sha256(&prior.hash, "prior manifest hash")?;
    let prior_path = resolve_input_path(repository, &prior.path);
    let prior_bytes = bounded_file::read_regular(
        &prior_path,
        MANIFEST_LIMIT,
        "prior published review manifest",
    )?;
    require_exact_hash(&prior.hash, &prior_bytes, "prior published review manifest")?;
    let prior_manifest: ManifestHeader = serde_json::from_slice(&prior_bytes)
        .map_err(|error| format!("invalid prior published review manifest: {error}"))?;
    if !review_packet::is_original_manifest_schema(&prior_manifest.schema) {
        return Err(
            "standing authorization allows at most one direct finding-resolution request"
                .to_owned(),
        );
    }
    let review_id = prior_manifest
        .review_id
        .ok_or_else(|| "prior original review manifest has no review_id".to_owned())?;
    require_sha256(&review_id, "prior original ReviewId")?;
    require_sha256(&prior_manifest.packet.hash, "prior original packet hash")?;
    Ok(ReviewClassification {
        kind: ReviewKind::FindingResolution,
        prior: Some(PriorReview {
            review_id,
            packet_hash: prior_manifest.packet.hash,
        }),
    })
}

fn capture_prior_delivery(
    repository: &Path,
    request: &Request,
    classification: &ReviewClassification,
) -> Result<Option<CapturedDeliveryReceipt>, String> {
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
        "prior external review delivery receipt",
    )?;
    require_exact_hash(
        &reference.hash,
        &bytes,
        "prior external review delivery receipt",
    )?;
    let receipt: DeliveryReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid prior external review delivery receipt: {error}"))?;
    if receipt.schema != DELIVERY_RECEIPT_SCHEMA {
        return Err(format!(
            "unsupported prior delivery receipt schema `{}`; expected `{DELIVERY_RECEIPT_SCHEMA}`",
            receipt.schema
        ));
    }
    require_sha256(&receipt.review_id, "prior delivery ReviewId")?;
    require_sha256(&receipt.packet_hash, "prior delivery packet hash")?;
    validate_route(&receipt.route)?;
    compact_token(
        &receipt.session_id,
        MAX_SESSION_ID_BYTES,
        "prior delivery session id",
    )?;
    compact_token(
        &receipt.provider_request_id,
        256,
        "prior delivery provider request id",
    )?;
    if receipt.provider_request_count != 1 {
        return Err("prior delivery receipt must record exactly one provider request".to_owned());
    }
    if receipt.review_id != prior.review_id || receipt.packet_hash != prior.packet_hash {
        return Err("prior delivery receipt does not match the original review packet".to_owned());
    }
    if receipt.route != request.route {
        return Err("finding-resolution route differs from the original delivery route".to_owned());
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
    Ok(Some(CapturedDeliveryReceipt { path, bytes }))
}

pub(crate) fn verify_completed_delivery(
    repository: &Path,
    receipt_path: &Path,
    review: &VerifiedReview,
) -> Result<VerifiedDeliveryRoute, String> {
    let path = resolve_input_path(repository, &receipt_path.to_string_lossy());
    let bytes = bounded_file::read_regular(
        &path,
        DELIVERY_RECEIPT_LIMIT,
        "external review delivery receipt",
    )?;
    let receipt = parse_delivery_receipt(&bytes, "external review delivery receipt")?;
    require_delivery_matches(
        &receipt,
        &review.review_id,
        &review.packet_hash,
        "external review delivery receipt does not match the reviewed packet",
    )?;
    Ok(VerifiedDeliveryRoute {
        provider: receipt.route.provider,
        model: receipt.route.model,
        session_id: receipt.session_id,
    })
}

fn parse_delivery_receipt(bytes: &[u8], label: &str) -> Result<DeliveryReceipt, String> {
    let receipt: DeliveryReceipt =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid {label}: {error}"))?;
    if receipt.schema != DELIVERY_RECEIPT_SCHEMA {
        return Err(format!(
            "unsupported delivery receipt schema `{}`; expected `{DELIVERY_RECEIPT_SCHEMA}`",
            receipt.schema
        ));
    }
    require_sha256(&receipt.review_id, "delivery ReviewId")?;
    require_sha256(&receipt.packet_hash, "delivery packet hash")?;
    validate_route(&receipt.route)?;
    compact_token(
        &receipt.session_id,
        MAX_SESSION_ID_BYTES,
        "delivery session id",
    )?;
    compact_token(
        &receipt.provider_request_id,
        256,
        "delivery provider request id",
    )?;
    if receipt.provider_request_count != 1 {
        return Err("delivery receipt must record exactly one provider request".to_owned());
    }
    Ok(receipt)
}

fn require_delivery_matches(
    receipt: &DeliveryReceipt,
    review_id: &str,
    packet_hash: &str,
    mismatch: &str,
) -> Result<(), String> {
    if receipt.review_id != review_id || receipt.packet_hash != packet_hash {
        Err(mismatch.to_owned())
    } else {
        Ok(())
    }
}

fn final_revalidate(
    repository: &Path,
    inputs: &FinalRevalidation<'_>,
    verify: &dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String>,
) -> Result<(), String> {
    for (path, expected, limit, label) in [
        (
            inputs.request_path,
            inputs.request_bytes,
            REQUEST_LIMIT,
            "Slice review egress request",
        ),
        (
            inputs.authorization_path,
            inputs.authorization_bytes,
            AUTHORIZATION_LIMIT,
            "external review standing authorization",
        ),
    ] {
        let current = bounded_file::read_regular(path, limit, label)?;
        if current != expected {
            return Err(format!("{label} changed during egress authorization"));
        }
    }
    if let Some(receipt) = inputs.prior_delivery {
        let current = bounded_file::read_regular(
            &receipt.path,
            DELIVERY_RECEIPT_LIMIT,
            "prior external review delivery receipt",
        )?;
        if current != receipt.bytes {
            return Err(
                "prior external review delivery receipt changed during egress authorization"
                    .to_owned(),
            );
        }
    }
    let current = verify(repository, inputs.manifest_path, inputs.manifest_hash)?;
    if current != *inputs.expected_review {
        return Err("verified review chain changed during final revalidation".to_owned());
    }
    Ok(())
}

fn validate_route(route: &Route) -> Result<(), String> {
    compact_token(&route.provider, MAX_ROUTE_TOKEN_BYTES, "route provider")?;
    compact_token(&route.account, MAX_ROUTE_TOKEN_BYTES, "route account")?;
    compact_token(&route.model, MAX_ROUTE_TOKEN_BYTES, "route model")
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

fn compact_token(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_whitespace())
    {
        Err(format!(
            "{label} must be a non-empty visible ASCII token of at most {max} bytes"
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

fn require_exact_hash(expected: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    require_sha256(expected, &format!("{label} hash"))?;
    let actual = digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}
