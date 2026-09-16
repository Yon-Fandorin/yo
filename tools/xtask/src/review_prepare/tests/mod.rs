mod authority;
mod request;
mod revalidation;
mod target;

use std::{fs, path::PathBuf};

use serde_json::json;

use super::model::Request;
use crate::{review_packet::PublishedReview, test_support::unique_path};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let path = unique_path(label);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn published_review() -> PublishedReview {
    PublishedReview {
        status: "created",
        schema: "yo.slice-review-packet-result/v1",
        authority: None,
        review_id: format!("sha256:{}", "1".repeat(64)),
        trusted_commit: "2".repeat(40),
        candidate_commit: "3".repeat(40),
        packet_path: ".local-exclude/methexis/slice-reviews/review/packet.md".to_owned(),
        packet_hash: format!("sha256:{}", "4".repeat(64)),
        packet_bytes: 123,
        managed_payload_tokens: 45,
        manifest_path: ".local-exclude/methexis/slice-reviews/review/manifest.json".to_owned(),
        manifest_hash: format!("sha256:{}", "5".repeat(64)),
        max_managed_payload_tokens: 1000,
    }
}

fn sample_request(target: serde_json::Value) -> Request {
    serde_json::from_value(json!({
        "schema": "yo.slice-review-prepare-request/v1alpha1",
        "slice": "review-preparation",
        "knowledge_ids": ["methexis.review.bounded-packet"],
        "context_max_tokens": 16000,
        "repository_authority_paths": ["CONTRIBUTING.md"],
        "validation_evidence": [{"name": "xtask", "path": "/tmp/xtask.json"}],
        "review_lenses": ["fresh-context", "code-quality"],
        "review_questions": ["Is the boundary exact?"],
        "max_managed_payload_tokens": 100000,
        "target": target
    }))
    .unwrap()
}
