use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{CandidateReference, CandidateSet, capture, final_revalidate_after_read, validation};
use crate::context::hash::digest;

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

#[test]
fn candidate_paths_reject_dot_and_empty_raw_components() {
    assert!(!super::capture::safe_relative(Path::new("a/./b.json")));
    assert!(!super::capture::safe_relative(Path::new("a//b.json")));
    assert!(super::capture::safe_relative(Path::new("a/b.json")));
}

fn librarian_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("librarian/examples/discovery-contract/expected-query-english.json")
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
