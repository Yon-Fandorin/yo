use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::{
    VerifiedDeliveryRoute,
    model::{
        AUTHORIZATION_SCHEMA, Authorization, DELIVERY_RECEIPT_SCHEMA, DeliveryLimits,
        DeliveryReceipt, ManifestHeader, PacketResult, REQUEST_SCHEMA, RESULT_SCHEMA, Request,
        ResultDocument, ReviewKind, Route, Session,
    },
    validation::{
        AUTHORIZATION_LIMIT, DELIVERY_RECEIPT_LIMIT, MANIFEST_LIMIT, MAX_AUTHORIZED_TOKENS,
        MAX_ROUTES, MAX_SESSION_ID_BYTES, PACKET_LIMIT, REQUEST_LIMIT, ReviewClassification,
        classify_review_kind, compact_path, compact_token, require_exact_hash, require_sha256,
        validate_route,
    },
};
use crate::{
    bounded_file, git, review_delta,
    review_packet::VerifiedReview,
    review_protocol::{digest, resolve_input_path},
};

#[derive(Debug)]
struct CapturedDeliveryReceipt {
    path: PathBuf,
    bytes: Vec<u8>,
    provider_request_id: String,
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
    pub(crate) session_id: Option<String>,
    pub(crate) prior_packet_hash: Option<String>,
    pub(crate) prior_provider_request_id: Option<String>,
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

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
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
        classification.finding_resolution_request_index,
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
        session_id: match &request.session {
            Session::Fresh => None,
            Session::Resume { id } => Some(id.clone()),
        },
        prior_packet_hash: classification
            .prior
            .as_ref()
            .map(|prior| prior.packet_hash.clone()),
        prior_provider_request_id: prior_delivery
            .as_ref()
            .map(|delivery| delivery.provider_request_id.clone()),
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
    finding_resolution_request_index: usize,
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
            if route.allow_finding_resolution_resume && finding_resolution_request_index == 1 => {},
        (ReviewKind::Original, Session::Fresh) => {
            return Err("the route does not authorize an original fresh review".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Resume { .. })
            if !route.allow_finding_resolution_resume =>
        {
            return Err("the route does not authorize a finding-resolution resume".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Resume { .. }) => {
            return Err(
                "standing authorization allows at most one direct finding-resolution request"
                    .to_owned(),
            );
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
    Ok(Some(CapturedDeliveryReceipt {
        path,
        bytes,
        provider_request_id: receipt.provider_request_id,
    }))
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
    Ok(VerifiedDeliveryRoute::Managed {
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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs, path::PathBuf, str};

    use super::{
        super::{
            model::{
                AUTHORIZATION_SCHEMA, Artifact, Authorization, AuthorizedRoute, ManifestHeader,
                ManifestInputs, PacketRecord, REQUEST_SCHEMA, Request, ReviewKind, Route, Session,
            },
            validation::{PriorReview, ReviewClassification, classify_review_kind},
        },
        authorize, canonical_authorization_path, capture_prior_delivery, evaluate_with,
        validate_authorization, validate_request,
    };
    use crate::{
        review_packet::VerifiedReview, review_protocol::digest, test_support::TestRepository,
    };

    fn hash(byte: u8) -> String {
        format!("sha256:{}", format!("{byte:02x}").repeat(32))
    }

    fn route() -> Route {
        Route {
            provider: "qwencloud".to_owned(),
            account: "default".to_owned(),
            model: "qwen3.8-max".to_owned(),
        }
    }

    fn request(session: Session) -> Request {
        Request {
            schema: REQUEST_SCHEMA.to_owned(),
            manifest_path: ".local-exclude/review/manifest.json".to_owned(),
            manifest_hash: hash(1),
            authorization_hash: hash(2),
            route: route(),
            session,
            prior_delivery: None,
        }
    }

    fn authorization() -> Authorization {
        Authorization {
            schema: AUTHORIZATION_SCHEMA.to_owned(),
            authority: "human/yon".to_owned(),
            status: "active".to_owned(),
            routes: vec![AuthorizedRoute {
                provider: "qwencloud".to_owned(),
                account: "default".to_owned(),
                model: "qwen3.8-max".to_owned(),
                max_packet_bytes: 1_000_000,
                max_managed_payload_tokens: 200_000,
                allow_original_fresh: true,
                allow_finding_resolution_resume: true,
            }],
        }
    }

    // 한 번 승인된 exact route는 원본 fresh와 직접 finding-resolution resume만 허용하며
    // 허용량 안의 packet에서는 추가 자연어 승인을 요구하지 않는다.
    #[test]
    fn exact_route_authorizes_the_two_bounded_review_kinds() {
        let authorization = authorization();
        let original = request(Session::Fresh);
        authorize(
            &original,
            &authorization,
            ReviewKind::Original,
            0,
            900_000,
            190_000,
        )
        .unwrap();

        let continuation = request(Session::Resume {
            id: "01a027bb-0d83-7b92-84ee-c3e2eb527d05".to_owned(),
        });
        authorize(
            &continuation,
            &authorization,
            ReviewKind::FindingResolution,
            1,
            40_000,
            12_000,
        )
        .unwrap();
    }

    // standing authorization은 비슷한 provider 이름이나 같은 모델의 다른 account를
    // 포괄하지 않으며 byte/token 한계를 각각 독립적으로 닫는다.
    #[test]
    fn route_and_packet_limits_fail_closed() {
        let authorization = authorization();
        let mut wrong = request(Session::Fresh);
        wrong.route.account = "other".to_owned();
        assert_eq!(
            authorize(&wrong, &authorization, ReviewKind::Original, 0, 1, 1).unwrap_err(),
            "requested external review route is not authorized"
        );

        let exact = request(Session::Fresh);
        assert!(
            authorize(
                &exact,
                &authorization,
                ReviewKind::Original,
                0,
                1_000_001,
                1
            )
            .unwrap_err()
            .contains("byte route limit")
        );
        assert!(
            authorize(&exact, &authorization, ReviewKind::Original, 0, 1, 200_001)
                .unwrap_err()
                .contains("token route limit")
        );
    }

    // 원본을 기존 Session에 보내거나 delta를 새 Session에 보내면 리뷰 문맥과 요청 횟수
    // 의미가 달라지므로 route 권한이 있어도 거부한다.
    #[test]
    fn session_mode_must_match_review_kind() {
        let authorization = authorization();
        let resume = request(Session::Resume {
            id: "existing-session".to_owned(),
        });
        assert_eq!(
            authorize(&resume, &authorization, ReviewKind::Original, 0, 1, 1).unwrap_err(),
            "an original review requires a fresh Session"
        );

        let fresh = request(Session::Fresh);
        assert_eq!(
            authorize(
                &fresh,
                &authorization,
                ReviewKind::FindingResolution,
                1,
                1,
                1,
            )
            .unwrap_err(),
            "a finding-resolution review requires the existing reviewer Session"
        );
    }

    // agent가 만든 일반 파일은 standing authority가 될 수 없고 비활성·중복·무효 route도
    // 승인 범위를 넓히는 입력으로 사용할 수 없다.
    #[test]
    fn standing_authorization_requires_active_human_origin_and_unique_routes() {
        let mut value = authorization();
        validate_authorization(&value).unwrap();

        value.authority = "codex/session".to_owned();
        assert!(
            validate_authorization(&value)
                .unwrap_err()
                .contains("human/")
        );
        value.authority = "human/".to_owned();
        assert!(
            validate_authorization(&value)
                .unwrap_err()
                .contains("human owner")
        );
        value.authority = "human/yon".to_owned();
        value.status = "revoked".to_owned();
        assert!(
            validate_authorization(&value)
                .unwrap_err()
                .contains("not active")
        );
        value.status = "active".to_owned();
        value.routes.push(AuthorizedRoute {
            provider: "qwencloud".to_owned(),
            account: "default".to_owned(),
            model: "qwen3.8-max".to_owned(),
            max_packet_bytes: 1,
            max_managed_payload_tokens: 1,
            allow_original_fresh: true,
            allow_finding_resolution_resume: false,
        });
        assert!(
            validate_authorization(&value)
                .unwrap_err()
                .contains("unique")
        );
    }

    // review chain은 immediate prior identity를 보존하며 request index를 계산하고, direct
    // standing authorization은 별도 다중-hop 권한이 없으므로 두 번째 resolution을 거부한다.
    #[test]
    fn recursive_classification_counts_resolution_requests_but_direct_authorization_stays_bounded()
    {
        let repository = TestRepository::new("review-egress-depth");
        let original_text = format!(
            "{{\"schema\":\"yo.slice-review-manifest/v1\",\"review_id\":\"{}\",\"packet\":{{\"hash\":\"{}\",\"managed_payload_tokens\":1}}}}\n",
            hash(5),
            hash(6)
        );
        let original_bytes = original_text.as_bytes();
        let original_path =
            repository.write(".local-exclude/original/manifest.json", &original_text);
        let direct = ManifestHeader {
            schema: "yo.slice-review-delta-manifest/v1".to_owned(),
            review_id: None,
            review_delta_id: Some(hash(8)),
            packet: PacketRecord {
                hash: hash(3),
                managed_payload_tokens: 1,
            },
            inputs: Some(ManifestInputs {
                prior_manifest: Some(Artifact {
                    path: original_path.to_string_lossy().into_owned(),
                    hash: digest(original_bytes),
                }),
            }),
        };
        let direct_classification = classify_review_kind(&repository.path, &direct).unwrap();
        assert_eq!(direct_classification.kind, ReviewKind::FindingResolution);
        assert_eq!(direct_classification.finding_resolution_request_index, 1);

        let direct_text = format!(
            "{{\"schema\":\"yo.slice-review-delta-manifest/v1\",\"review_delta_id\":\"{}\",\"packet\":{{\"hash\":\"{}\",\"managed_payload_tokens\":1}},\"inputs\":{{\"prior_manifest\":{{\"path\":\"{}\",\"hash\":\"{}\"}}}}}}\n",
            hash(8),
            hash(3),
            original_path.display(),
            digest(original_bytes),
        );
        let direct_bytes = direct_text.as_bytes();
        let direct_path = repository.write(".local-exclude/delta/manifest.json", &direct_text);
        let nested = ManifestHeader {
            schema: "yo.slice-review-delta-manifest/v1".to_owned(),
            review_id: None,
            review_delta_id: Some(hash(9)),
            packet: PacketRecord {
                hash: hash(4),
                managed_payload_tokens: 1,
            },
            inputs: Some(ManifestInputs {
                prior_manifest: Some(Artifact {
                    path: direct_path.to_string_lossy().into_owned(),
                    hash: digest(direct_bytes),
                }),
            }),
        };
        let nested_classification = classify_review_kind(&repository.path, &nested).unwrap();
        assert_eq!(nested_classification.finding_resolution_request_index, 2);
        assert_eq!(nested_classification.prior.unwrap().review_id, hash(8));
        let continuation = request(Session::Resume {
            id: "existing-session".to_owned(),
        });
        assert!(
            authorize(
                &continuation,
                &authorization(),
                ReviewKind::FindingResolution,
                2,
                1,
                1,
            )
            .unwrap_err()
            .contains("at most one direct")
        );
    }

    // request의 path·hash·route·resume identity는 모두 bounded exact 입력이어야 하며 빈 값이나
    // 공백이 섞인 session identity를 전송 준비로 받아들이지 않는다.
    #[test]
    fn request_identity_is_bounded_before_artifact_reads() {
        let valid = request(Session::Fresh);
        validate_request(&valid).unwrap();

        let invalid = request(Session::Resume {
            id: "not a token".to_owned(),
        });
        assert!(
            validate_request(&invalid)
                .unwrap_err()
                .contains("visible ASCII")
        );
    }

    // 모든 worktree가 Git common directory의 부모에 있는 단 하나의 authorization 파일을
    // 사용하므로 이전 active bytes를 다른 경로로 복사해 revocation을 우회할 수 없다.
    #[test]
    fn authorization_path_is_shared_and_not_caller_selected() {
        let repository = TestRepository::new("review-egress-authorization-path");

        assert_eq!(
            canonical_authorization_path(&repository.path).unwrap(),
            repository
                .path
                .join(".local-exclude/authorizations/external-review.json")
        );
    }

    struct CommandFixture {
        repository: TestRepository,
        request_path: PathBuf,
        authorization_path: PathBuf,
        verified: VerifiedReview,
    }

    fn command_fixture(label: &str) -> CommandFixture {
        let repository = TestRepository::new(label);
        let packet = b"immutable packet\n";
        let packet_path = repository.write(
            ".local-exclude/review/packet.md",
            str::from_utf8(packet).unwrap(),
        );
        let review_id = hash(8);
        let packet_hash = digest(packet);
        let manifest_text = format!(
            "{{\"schema\":\"yo.slice-review-manifest/v1\",\"review_id\":\"{review_id}\",\"packet\":{{\"hash\":\"{packet_hash}\",\"managed_payload_tokens\":3}}}}\n"
        );
        let manifest_path = repository.write(".local-exclude/review/manifest.json", &manifest_text);
        let authorization = serde_json::json!({
            "schema": AUTHORIZATION_SCHEMA,
            "authority": "human/yon",
            "status": "active",
            "routes": [{
                "provider": "qwencloud",
                "account": "default",
                "model": "qwen3.8-max",
                "max_packet_bytes": 1000,
                "max_managed_payload_tokens": 1000,
                "allow_original_fresh": true,
                "allow_finding_resolution_resume": true
            }]
        });
        let authorization_text = format!(
            "{}\n",
            serde_json::to_string_pretty(&authorization).unwrap()
        );
        let authorization_path = repository.write(
            ".local-exclude/authorizations/external-review.json",
            &authorization_text,
        );
        let request = serde_json::json!({
            "schema": REQUEST_SCHEMA,
            "manifest_path": ".local-exclude/review/manifest.json",
            "manifest_hash": digest(manifest_text.as_bytes()),
            "authorization_hash": digest(authorization_text.as_bytes()),
            "route": {
                "provider": "qwencloud",
                "account": "default",
                "model": "qwen3.8-max"
            },
            "session": {"mode": "fresh"}
        });
        let request_path = repository.write(
            ".local-exclude/review/egress-request.json",
            &format!("{}\n", serde_json::to_string_pretty(&request).unwrap()),
        );
        let verified = VerifiedReview {
            review_id,
            manifest_path: manifest_path
                .strip_prefix(&repository.path)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            manifest_hash: digest(manifest_text.as_bytes()),
            packet_path: packet_path
                .strip_prefix(&repository.path)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            packet_hash,
            base_commit: "11".repeat(20),
            candidate_commit: "22".repeat(20),
            trusted_commit: "33".repeat(20),
            slice_contract_path: "slice-contract.json".to_owned(),
            slice_contract_hash: hash(9),
            validation_evidence: Vec::new(),
            review_lenses: vec!["fresh-context".to_owned()],
            review_questions: vec!["Is the boundary closed?".to_owned()],
        };
        CommandFixture {
            repository,
            request_path,
            authorization_path,
            verified,
        }
    }

    // command-level 평가가 canonical authorization, exact manifest와 packet, route 한계를 함께
    // 소비하고 final chain verification까지 같은 identity일 때만 deliver_once를 반환한다.
    #[test]
    fn command_evaluation_returns_one_bounded_delivery_action() {
        let fixture = command_fixture("review-egress-command");
        let expected = fixture.verified.clone();
        let output = evaluate_with(
            &fixture.repository.path,
            &fixture.request_path,
            &|_, _, _| Ok(expected.clone()),
        )
        .unwrap();

        assert_eq!(output.next_action, "deliver_once");
        assert_eq!(output.review_id, fixture.verified.review_id);
        assert_eq!(output.packet.hash, fixture.verified.packet_hash);
        assert_eq!(output.limits.provider_requests, 1);
        assert_eq!(output.limits.retries, 0);
        assert!(!output.limits.tool_execution);
    }

    // canonical authorization을 revoked bytes로 바꾸면 request가 옛 hash를 갖고 있어도 verifier
    // 호출 전에 멈추므로 복사본이나 stale request가 이전 권한을 되살리지 못한다.
    #[test]
    fn command_evaluation_observes_canonical_authorization_revocation() {
        let fixture = command_fixture("review-egress-revoked");
        let current = fs::read_to_string(&fixture.authorization_path).unwrap();
        fs::write(
            &fixture.authorization_path,
            current.replace("\"active\"", "\"revoked\""),
        )
        .unwrap();

        let error = evaluate_with(
            &fixture.repository.path,
            &fixture.request_path,
            &|_, _, _| panic!("review verifier must not run after authorization revocation"),
        )
        .unwrap_err();
        assert!(error.contains("standing authorization hash mismatch"));
    }

    // 최초 replay 뒤 prior chain이나 trusted Git identity가 달라지면 final verifier의 complete
    // 결과 비교가 변화를 잡아 authorized 결과를 내지 않는다.
    #[test]
    fn command_evaluation_replays_the_complete_chain_at_final_revalidation() {
        let fixture = command_fixture("review-egress-final-revalidation");
        let calls = Cell::new(0);
        let expected = fixture.verified.clone();
        let error = evaluate_with(
            &fixture.repository.path,
            &fixture.request_path,
            &|_, _, _| {
                let call = calls.get();
                calls.set(call + 1);
                let mut observed = expected.clone();
                if call == 1 {
                    observed.trusted_commit = "44".repeat(20);
                }
                Ok(observed)
            },
        )
        .unwrap_err();

        assert_eq!(calls.get(), 2);
        assert_eq!(
            error,
            "verified review chain changed during final revalidation"
        );
    }

    // finding-resolution은 original ReviewId, packet hash, exact route, 실제 Session과 request
    // identity를 담은 1회 delivery receipt 없이는 resume 권한을 얻지 못한다.
    #[test]
    fn finding_resolution_binds_the_original_delivery_receipt() {
        let repository = TestRepository::new("review-egress-prior-delivery");
        let review_id = hash(10);
        let packet_hash = hash(11);
        let receipt = serde_json::json!({
            "schema": "yo.external-review-delivery-receipt/v1",
            "review_id": review_id,
            "packet_hash": packet_hash,
            "route": {
                "provider": "qwencloud",
                "account": "default",
                "model": "qwen3.8-max"
            },
            "session_id": "review-session",
            "provider_request_id": "request-1",
            "provider_request_count": 1
        });
        let receipt_text = format!("{}\n", serde_json::to_string_pretty(&receipt).unwrap());
        let receipt_path = repository.write(".local-exclude/review/delivery.json", &receipt_text);
        let mut request = request(Session::Resume {
            id: "review-session".to_owned(),
        });
        request.prior_delivery = Some(Artifact {
            path: receipt_path.to_string_lossy().into_owned(),
            hash: digest(receipt_text.as_bytes()),
        });
        let classification = ReviewClassification {
            kind: ReviewKind::FindingResolution,
            finding_resolution_request_index: 1,
            prior: Some(PriorReview {
                review_id,
                packet_hash,
            }),
        };

        capture_prior_delivery(&repository.path, &request, &classification).unwrap();
        request.session = Session::Resume {
            id: "unrelated-session".to_owned(),
        };
        assert!(
            capture_prior_delivery(&repository.path, &request, &classification)
                .unwrap_err()
                .contains("differs from the original delivery Session")
        );
    }
}
