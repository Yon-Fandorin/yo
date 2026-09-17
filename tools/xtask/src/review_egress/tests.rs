use super::VerifiedDeliveryRoute;
use crate::{review_packet::VerifiedReview, test_support::TestRepository};

fn hash(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn review(review_id: &str, packet_hash: &str) -> VerifiedReview {
    VerifiedReview {
        review_id: review_id.to_owned(),
        manifest_path: "manifest.json".to_owned(),
        manifest_hash: hash(3),
        packet_path: "packet.md".to_owned(),
        packet_hash: packet_hash.to_owned(),
        base_commit: "11".repeat(20),
        candidate_commit: "22".repeat(20),
        trusted_commit: "33".repeat(20),
        slice_contract_path: "slice-contract.json".to_owned(),
        slice_contract_hash: hash(4),
        validation_evidence: Vec::new(),
        review_lenses: vec!["fresh-context".to_owned()],
        review_questions: vec!["Does the facade preserve the receipt route?".to_owned()],
    }
}

// facade는 schema만으로 managed와 delegated 구현을 선택하되 각 protocol의 오류 경계를
// 그대로 보존해 잘못된 요청이 반대 owner의 parser로 흘러가지 않게 합니다.
#[test]
fn dispatches_each_request_schema_to_its_protocol_owner() {
    let repository = TestRepository::new("review-egress-facade-dispatch");
    let delegated = repository.write(
        ".local-exclude/review/delegated-request.json",
        r#"{"schema":"yo.slice-review-delegated-egress-request/v1alpha1"}"#,
    );
    let error = super::run(&repository.path, &delegated).unwrap_err();
    assert!(error.contains("invalid delegated Slice review egress request"));

    let managed = repository.write(
        ".local-exclude/review/managed-request.json",
        r#"{"schema":"yo.slice-review-egress-request/v1"}"#,
    );
    let error = super::run(&repository.path, &managed).unwrap_err();
    assert!(error.contains("invalid Slice review egress request"));
}

// 공통 verify facade는 receipt schema를 기준으로 managed route와 delegated host route를
// 각각의 owner verifier에 보내고, caller에게는 동일한 VerifiedDeliveryRoute만 노출합니다.
#[test]
fn verifies_managed_and_delegated_receipts_through_the_common_facade() {
    let repository = TestRepository::new("review-egress-facade-receipts");
    let managed_review_id = hash(5);
    let managed_packet_hash = hash(6);
    let managed_review = review(&managed_review_id, &managed_packet_hash);
    let managed_receipt = repository.write(
        ".local-exclude/review/managed-receipt.json",
        &serde_json::to_string(&serde_json::json!({
            "schema": "yo.external-review-delivery-receipt/v1",
            "review_id": managed_review_id,
            "packet_hash": managed_packet_hash,
            "route": {
                "provider": "qwencloud",
                "account": "default",
                "model": "qwen3.8-max"
            },
            "session_id": "managed-session",
            "provider_request_id": "provider-request",
            "provider_request_count": 1
        }))
        .unwrap(),
    );
    assert_eq!(
        super::verify_any_completed_delivery(&repository.path, &managed_receipt, &managed_review)
            .unwrap(),
        VerifiedDeliveryRoute::Managed {
            provider: "qwencloud".to_owned(),
            model: "qwen3.8-max".to_owned(),
            session_id: "managed-session".to_owned(),
        }
    );

    let delegated_review_id = hash(7);
    let delegated_packet_hash = hash(8);
    let delegated_review = review(&delegated_review_id, &delegated_packet_hash);
    let delegated_receipt = repository.write(
        ".local-exclude/review/delegated-receipt.json",
        &serde_json::to_string(&serde_json::json!({
            "schema": "yo.external-review-delegated-delivery-receipt/v1alpha1",
            "review_id": delegated_review_id,
            "packet_hash": delegated_packet_hash,
            "target": {"kind": "delegated_host", "host": "codex"},
            "execution_profile": "yo.delegated-review-execution/v1alpha1",
            "session_id": "delegated-session",
            "host_request_id": "host-request",
            "host_request_count": 1
        }))
        .unwrap(),
    );
    assert_eq!(
        super::verify_any_completed_delivery(
            &repository.path,
            &delegated_receipt,
            &delegated_review,
        )
        .unwrap(),
        VerifiedDeliveryRoute::Delegated {
            host: "codex".to_owned(),
            session_id: "delegated-session".to_owned(),
        }
    );
}
