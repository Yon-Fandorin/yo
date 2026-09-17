use std::{fs, path::Path};

use super::{
    bounds::{END, START},
    correction_preflight,
    parse::inspect,
    verify,
};
use crate::test_support::unique_path;

fn response(json: &str) -> Vec<u8> {
    format!(
        "review notes\n{}\n{json}\n{}\n",
        String::from_utf8_lossy(START),
        String::from_utf8_lossy(END)
    )
    .into_bytes()
}

fn lenses() -> Vec<String> {
    vec!["code-quality".to_owned(), "fresh-context".to_owned()]
}

// terminal envelope 하나가 exact review/candidate와 모든 lens를 묶을 때만 gate가
// 사람이 다시 적은 verdict 없이 동일한 구조화 결과를 소비할 수 있습니다.
#[test]
fn accepts_exact_terminal_result() {
    let bytes = response(
        r#"{"schema":"yo.slice-review-result/v1alpha1","review_id":"sha256:review","candidate_commit":"candidate","verdicts":[{"lens":"fresh-context","verdict":"findings"},{"lens":"code-quality","verdict":"clear"}],"findings":[{"finding_id":"F1","summary":"Missing final guard.","lenses":["fresh-context"]}]}"#,
    );
    let result = verify(&bytes, "sha256:review", "candidate", &lenses()).unwrap();
    assert_eq!(result.verdicts[0].lens, "code-quality");
    assert_eq!(result.findings[0].finding_id, "F1");
}

// 결과 뒤 prose나 요청하지 않은 lens를 허용하면 모델 출력 일부만 골라 gate를
// 통과시킬 수 있으므로 terminal framing과 exact lens 집합을 함께 닫습니다.
#[test]
fn rejects_trailing_output_and_lens_drift() {
    let mut trailing = response(
        r#"{"schema":"yo.slice-review-result/v1alpha1","review_id":"sha256:review","candidate_commit":"candidate","verdicts":[{"lens":"code-quality","verdict":"clear"},{"lens":"fresh-context","verdict":"clear"}],"findings":[]}"#,
    );
    trailing.extend_from_slice(b"not terminal");
    assert!(
        verify(&trailing, "sha256:review", "candidate", &lenses())
            .unwrap_err()
            .contains("terminal")
    );

    let drift = response(
        r#"{"schema":"yo.slice-review-result/v1alpha1","review_id":"sha256:review","candidate_commit":"candidate","verdicts":[{"lens":"code-quality","verdict":"clear"}],"findings":[]}"#,
    );
    assert!(
        verify(&drift, "sha256:review", "candidate", &lenses())
            .unwrap_err()
            .contains("every and only")
    );
}

// findings verdict와 실제 finding lens 집합이 다르면 clear를 주장하면서 지적을
// 숨기거나 근거 없는 findings 상태를 만들 수 있어 양방향 일치를 요구합니다.
#[test]
fn rejects_inconsistent_finding_set() {
    let bytes = response(
        r#"{"schema":"yo.slice-review-result/v1alpha1","review_id":"sha256:review","candidate_commit":"candidate","verdicts":[{"lens":"code-quality","verdict":"clear"},{"lens":"fresh-context","verdict":"findings"}],"findings":[]}"#,
    );
    assert!(
        verify(&bytes, "sha256:review", "candidate", &lenses())
            .unwrap_err()
            .contains("explain every and only")
    );
}

// correction preflight는 verdict 의미가 온전하고 identity envelope만 틀린 응답만
// 식별해야 하므로 semantic 검증과 exact identity 검증을 분리합니다.
#[test]
fn inspection_preserves_semantics_while_exposing_identity_drift() {
    let bytes = response(
        r#"{"schema":"yo.slice-review-result/v1alpha1","review_id":"sha256:wrong","candidate_commit":"wrong-candidate","verdicts":[{"lens":"code-quality","verdict":"clear"},{"lens":"fresh-context","verdict":"clear"}],"findings":[]}"#,
    );
    let inspected = inspect(&bytes, &lenses()).unwrap();
    assert_eq!(inspected.review_id, "sha256:wrong");
    assert_eq!(inspected.candidate_commit, "wrong-candidate");
    assert!(
        verify(&bytes, "sha256:review", "candidate", &lenses())
            .unwrap_err()
            .contains("chain head")
    );
}

// 보정 요청 schema가 틀리면 repository 경로를 읽기 전에 중단하여 잘못된 보정
// 계약이 외부 artifact 검증으로 진행되지 않게 합니다.
#[test]
fn rejects_correction_request_schema_before_repository_access() {
    let request_path = unique_path("review-result-correction-schema");
    let request = serde_json::json!({
        "schema": "wrong-schema",
        "manifest_path": "manifest.json",
        "manifest_hash": format!("sha256:{}", "0".repeat(64)),
        "delivery_receipt_path": "receipt.json",
        "delivery_receipt_hash": format!("sha256:{}", "1".repeat(64)),
        "review_result_path": "result.txt",
        "review_result_hash": format!("sha256:{}", "2".repeat(64))
    });
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let error = correction_preflight(Path::new("."), &request_path).unwrap_err();
    let _ = fs::remove_file(&request_path);
    assert!(error.contains("unsupported review-result correction preflight schema"));
}
