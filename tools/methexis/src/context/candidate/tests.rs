use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{CandidateReference, CandidateSet, capture, final_revalidate_after_read, validation};
use crate::context::{
    hash::{StableHasher, digest},
    wire::CandidateReason,
};

const RESULT_ID_DOMAIN: &[u8] = b"librarian.candidate-set/v1alpha1";

// candidate를 선택한 뒤 파일이 바뀌면 새 내용으로 선택 결과를 몰래 다시 해석하지 않는다.
// 같은 snapshot을 유지할 수 없으므로 candidate_changed_during_resolution로 다시 시도하게 한다.
#[test]
fn concurrent_candidate_change_is_retryable_and_never_reinterpreted() {
    let fixture = librarian_fixture();
    let root = temporary_root();
    let path = root.join("candidate.json");
    let bytes = fs::read(fixture).unwrap();
    fs::write(&path, &bytes).unwrap();
    let captured = capture(
        &root,
        &CandidateReference {
            path: "candidate.json".to_owned(),
            hash: digest(&bytes),
        },
    )
    .unwrap();

    let failure = final_revalidate_after_read(&root, &captured, || {
        fs::write(&path, b"changed during validation\n").unwrap();
    })
    .unwrap_err();

    assert_eq!(failure.code(), "candidate_changed_during_resolution");
    fs::remove_dir_all(root).unwrap();
}

// Librarian 결과를 소비하는 독립 decoder가 변조된 score·순서·중복 candidate를 모두 잡아내는지
// 확인한다. 계약에 없는 필드도 무시하지 않아 producer와 consumer의 schema 차이를 조기에 드러낸다.
#[test]
fn independent_decoder_rejects_score_order_duplicate_and_unknown_field_drift() {
    let bytes = fs::read(librarian_fixture()).unwrap();
    let valid: CandidateSet = serde_json::from_slice(&bytes).unwrap();
    validation::validate(&valid, "candidate.json").unwrap();

    let mut score = valid.clone();
    score.candidates[0].score += 1;
    assert_eq!(
        validation::validate(&score, "candidate.json")
            .unwrap_err()
            .code(),
        "invalid_candidate_set"
    );

    let mut order = valid.clone();
    let mut lower_score_candidate = order.candidates[0].clone();
    lower_score_candidate.id = "tui.dependencies.selection-gate-secondary".to_owned();
    lower_score_candidate.path =
        "methexis/knowledge/tui-architecture/tui.dependencies.selection-gate-secondary.md"
            .to_owned();
    lower_score_candidate.reasons.truncate(1);
    lower_score_candidate.score = lower_score_candidate.reasons[0].score();
    order.candidates.push(lower_score_candidate);
    order.candidates.reverse();
    assert_eq!(
        validation::validate(&order, "candidate.json")
            .unwrap_err()
            .code(),
        "invalid_candidate_set"
    );

    let mut duplicate = valid.clone();
    duplicate.candidates.push(duplicate.candidates[0].clone());
    assert_eq!(
        validation::validate(&duplicate, "candidate.json")
            .unwrap_err()
            .code(),
        "invalid_candidate_set"
    );

    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown["unexpected"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<CandidateSet>(unknown).is_err());
}

// Librarian wire의 `relation` reason을 독립 decoder가 실제 enum으로 복원하고, 점수와
// candidate_set_id까지 일치하는 완전한 후보 집합으로 검증하는지 확인한다.
#[test]
fn independent_decoder_accepts_a_relation_wire_reason() {
    let bytes = fs::read(librarian_fixture()).unwrap();
    let mut wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    wire["candidates"][0]["reasons"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "kind": "relation",
            "via": "tui.architecture.module-boundaries",
            "score": 10
        }));
    let score = wire["candidates"][0]["score"].as_u64().unwrap();
    wire["candidates"][0]["score"] = serde_json::Value::from(score + 10);

    let mut decoded: CandidateSet = serde_json::from_value(wire).unwrap();
    refresh_candidate_set_id(&mut decoded);

    assert!(matches!(
        decoded.candidates[0].reasons.last(),
        Some(CandidateReason::Relation { via, score })
            if via == "tui.architecture.module-boundaries" && *score == 10
    ));
    validation::validate(&decoded, "candidate.json").unwrap();
}

// candidate 경로는 정규화 과정에서 다른 표기로 바뀌지 않는 명확한 상대 경로여야 한다.
// `.`이나 빈 구성 요소가 있는 경로는 거부하고 평범한 상대 경로만 허용한다.
#[test]
fn candidate_paths_reject_dot_and_empty_raw_components() {
    assert!(!capture::safe_relative(Path::new("a/./b.json")));
    assert!(!capture::safe_relative(Path::new("a//b.json")));
    assert!(capture::safe_relative(Path::new("a/b.json")));
}

fn librarian_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("librarian/examples/discovery-contract/expected-query-english.json")
}

fn refresh_candidate_set_id(set: &mut CandidateSet) {
    let candidate_bytes = serde_json::to_vec(&set.candidates).unwrap();
    let mut identity = StableHasher::new(RESULT_ID_DOMAIN);
    identity.part(b"request_hash", set.request_hash.as_bytes());
    identity.part(b"catalog_hash", set.catalog_hash.as_bytes());
    identity.part(b"compiler", set.compiler.as_bytes());
    identity.part(b"candidates", &candidate_bytes);
    set.candidate_set_id = identity.finish();
}

fn temporary_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-candidate-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    root
}
