use std::{cell::Cell, path::PathBuf};

use super::{
    PriorReview, ReviewClassification, authorize, canonical_authorization_path,
    capture_prior_delivery, classify_review_kind, evaluate_with,
    model::{
        AUTHORIZATION_SCHEMA, Artifact, Authorization, AuthorizedRoute, ManifestHeader,
        ManifestInputs, PacketRecord, REQUEST_SCHEMA, Request, ReviewKind, Route, Session,
    },
    validate_authorization, validate_request,
};
use crate::{review_packet::VerifiedReview, review_protocol::digest, test_support::TestRepository};

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
        authorize(&wrong, &authorization, ReviewKind::Original, 1, 1).unwrap_err(),
        "requested external review route is not authorized"
    );

    let exact = request(Session::Fresh);
    assert!(
        authorize(&exact, &authorization, ReviewKind::Original, 1_000_001, 1)
            .unwrap_err()
            .contains("byte route limit")
    );
    assert!(
        authorize(&exact, &authorization, ReviewKind::Original, 1, 200_001)
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
        authorize(&resume, &authorization, ReviewKind::Original, 1, 1).unwrap_err(),
        "an original review requires a fresh Session"
    );

    let fresh = request(Session::Fresh);
    assert_eq!(
        authorize(&fresh, &authorization, ReviewKind::FindingResolution, 1, 1).unwrap_err(),
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

// exact direct delta까지만 standing 범위로 분류하고, delta를 다시 잇는 세 번째 provider
// request는 별도 human 승인이 없으면 preflight 단계에서 멈춘다.
#[test]
fn only_one_direct_finding_resolution_is_standing_authorized() {
    let repository = crate::test_support::TestRepository::new("review-egress-depth");
    let original_text = format!(
        "{{\"schema\":\"yo.slice-review-manifest/v1\",\"review_id\":\"{}\",\"packet\":{{\"hash\":\"{}\",\"managed_payload_tokens\":1}}}}\n",
        hash(5),
        hash(6)
    );
    let original_bytes = original_text.as_bytes();
    let original_path = repository.write(".local-exclude/original/manifest.json", &original_text);
    let direct = ManifestHeader {
        schema: "yo.slice-review-delta-manifest/v1".to_owned(),
        review_id: None,
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
    assert_eq!(
        classify_review_kind(&repository.path, &direct)
            .unwrap()
            .kind,
        ReviewKind::FindingResolution
    );

    let nested_text = format!(
        "{{\"schema\":\"yo.slice-review-delta-manifest/v1\",\"packet\":{{\"hash\":\"{}\",\"managed_payload_tokens\":1}}}}\n",
        hash(7)
    );
    let nested_bytes = nested_text.as_bytes();
    let nested_path = repository.write(".local-exclude/delta/manifest.json", &nested_text);
    let nested = ManifestHeader {
        schema: "yo.slice-review-delta-manifest/v1".to_owned(),
        review_id: None,
        packet: PacketRecord {
            hash: hash(4),
            managed_payload_tokens: 1,
        },
        inputs: Some(ManifestInputs {
            prior_manifest: Some(Artifact {
                path: nested_path.to_string_lossy().into_owned(),
                hash: digest(nested_bytes),
            }),
        }),
    };
    assert!(
        classify_review_kind(&repository.path, &nested)
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
    let repository = crate::test_support::TestRepository::new("review-egress-authorization-path");

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
        std::str::from_utf8(packet).unwrap(),
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
    let current = std::fs::read_to_string(&fixture.authorization_path).unwrap();
    std::fs::write(
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
