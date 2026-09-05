use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{
    bounded_file,
    review::{delta as review_delta, egress as review_egress},
    review_protocol::{digest, resolve_input_path},
};

pub(crate) const SCHEMA: &str = "yo.slice-review-result/v1alpha1";
const START: &[u8] = b"<<<YO-SLICE-REVIEW-RESULT>>>";
const END: &[u8] = b"<<<YO-SLICE-REVIEW-RESULT-END>>>";
const MAX_FINDINGS: usize = 64;
const MAX_SUMMARY_BYTES: usize = 4096;
const CORRECTION_REQUEST_SCHEMA: &str =
    "yo.slice-review-result-correction-preflight-request/v1alpha1";
const CORRECTION_RESULT_SCHEMA: &str =
    "yo.slice-review-result-correction-preflight-result/v1alpha1";
const CORRECTION_REQUEST_LIMIT: usize = 64 * 1024;
const REVIEW_RESULT_LIMIT: usize = 64 * 1024;

pub(crate) const OUTPUT_INSTRUCTION: &str = r#"Finish the response with exactly one terminal structured result after any explanation. Copy the current packet's exact ReviewId (or ReviewDeltaId), candidate commit, and every requested lens exactly once. Use verdict `clear` or `findings`; list every material finding with a unique finding_id, bounded summary, and its affected lenses. Findings must be empty exactly when every lens is clear. Write nothing after the end marker:
<<<YO-SLICE-REVIEW-RESULT>>>
{"schema":"yo.slice-review-result/v1alpha1","review_id":"<current review id>","candidate_commit":"<current candidate>","verdicts":[{"lens":"<requested lens>","verdict":"clear"}],"findings":[]}
<<<YO-SLICE-REVIEW-RESULT-END>>>"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedResult {
    pub(crate) verdicts: Vec<Verdict>,
    pub(crate) findings: Vec<Finding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InspectedResult {
    review_id: String,
    candidate_commit: String,
    verified: VerifiedResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Verdict {
    pub(crate) lens: String,
    pub(crate) verdict: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Finding {
    pub(crate) finding_id: String,
    pub(crate) summary: String,
    pub(crate) lenses: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultDocument {
    schema: String,
    review_id: String,
    candidate_commit: String,
    verdicts: Vec<Verdict>,
    findings: Vec<Finding>,
}

pub(crate) fn verify(
    response: &[u8],
    expected_review_id: &str,
    expected_candidate: &str,
    expected_lenses: &[String],
) -> Result<VerifiedResult, String> {
    let inspected = inspect(response, expected_lenses)?;
    if inspected.review_id != expected_review_id {
        return Err(
            "structured review result does not identify the reviewed chain head".to_owned(),
        );
    }
    if inspected.candidate_commit != expected_candidate {
        return Err("structured review result does not identify the reviewed candidate".to_owned());
    }
    Ok(inspected.verified)
}

fn inspect(response: &[u8], expected_lenses: &[String]) -> Result<InspectedResult, String> {
    let start = exactly_one(response, START, "structured review result start marker")?;
    let json_start = start + START.len();
    let json_end = exactly_one(response, END, "structured review result end marker")?;
    if json_end < json_start {
        return Err("structured review result end marker precedes its start marker".to_owned());
    }
    if !response[json_end + END.len()..]
        .iter()
        .all(u8::is_ascii_whitespace)
    {
        return Err("structured review result must be the terminal review output".to_owned());
    }
    let document: ResultDocument =
        serde_json::from_slice(trim_ascii(&response[json_start..json_end]))
            .map_err(|error| format!("invalid structured review result: {error}"))?;
    if document.schema != SCHEMA {
        return Err(format!(
            "unsupported structured review result schema `{}`; expected `{SCHEMA}`",
            document.schema
        ));
    }
    let expected = expected_lenses
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if expected.len() != expected_lenses.len() || expected.is_empty() {
        return Err("expected review lenses must be non-empty and unique".to_owned());
    }
    let mut verdicts = BTreeMap::new();
    for verdict in document.verdicts {
        if !matches!(verdict.verdict.as_str(), "clear" | "findings") {
            return Err(format!(
                "structured review lens `{}` has unsupported verdict `{}`",
                verdict.lens, verdict.verdict
            ));
        }
        if verdicts.insert(verdict.lens.clone(), verdict).is_some() {
            return Err("structured review result contains a duplicate lens".to_owned());
        }
    }
    if verdicts.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected {
        return Err("structured review result must cover every and only requested lens".to_owned());
    }
    if document.findings.len() > MAX_FINDINGS {
        return Err(format!(
            "structured review result exceeds the {MAX_FINDINGS}-finding limit"
        ));
    }
    let mut finding_ids = BTreeSet::new();
    let mut lenses_with_findings = BTreeSet::new();
    for finding in &document.findings {
        compact_token(&finding.finding_id, 128, "finding_id")?;
        compact(&finding.summary, MAX_SUMMARY_BYTES, "finding summary")?;
        if !finding_ids.insert(finding.finding_id.as_str()) {
            return Err("structured review result contains a duplicate finding_id".to_owned());
        }
        let finding_lenses = finding
            .lenses
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if finding_lenses.len() != finding.lenses.len()
            || finding_lenses.is_empty()
            || !finding_lenses.is_subset(&expected)
        {
            return Err(
                "structured review finding lenses must be non-empty, unique, and requested"
                    .to_owned(),
            );
        }
        for lens in finding_lenses {
            if verdicts[lens].verdict != "findings" {
                return Err(format!(
                    "structured review finding names clear lens `{lens}`"
                ));
            }
            lenses_with_findings.insert(lens);
        }
    }
    let declared_findings = verdicts
        .values()
        .filter(|verdict| verdict.verdict == "findings")
        .map(|verdict| verdict.lens.as_str())
        .collect::<BTreeSet<_>>();
    if declared_findings != lenses_with_findings {
        return Err(
            "structured review findings must explain every and only lens with verdict findings"
                .to_owned(),
        );
    }

    Ok(InspectedResult {
        review_id: document.review_id,
        candidate_commit: document.candidate_commit,
        verified: VerifiedResult {
            verdicts: verdicts.into_values().collect(),
            findings: document.findings,
        },
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionRequest {
    schema: String,
    manifest_path: String,
    manifest_hash: String,
    delivery_receipt_path: String,
    delivery_receipt_hash: String,
    review_result_path: String,
    review_result_hash: String,
}

#[derive(Debug, Serialize)]
struct CorrectionResult {
    schema: &'static str,
    ok: bool,
    status: &'static str,
    next_action: &'static str,
    provider_requests: usize,
    expected_review_id: String,
    observed_review_id: String,
    expected_candidate_commit: String,
    observed_candidate_commit: String,
    session_id: String,
    route: String,
    immutable_result_hash: String,
}

pub(crate) fn correction_preflight(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        CORRECTION_REQUEST_LIMIT,
        "review-result correction preflight request",
    )?;
    let request: CorrectionRequest = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid review-result correction preflight request: {error}"))?;
    if request.schema != CORRECTION_REQUEST_SCHEMA {
        return Err(format!(
            "unsupported review-result correction preflight schema `{}`; expected `{CORRECTION_REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    for (value, label) in [
        (&request.manifest_hash, "manifest_hash"),
        (&request.delivery_receipt_hash, "delivery_receipt_hash"),
        (&request.review_result_hash, "review_result_hash"),
    ] {
        require_sha256(value, label)?;
    }
    let manifest_path = resolve_input_path(repository, &request.manifest_path);
    let receipt_path = resolve_input_path(repository, &request.delivery_receipt_path);
    let result_path = resolve_input_path(repository, &request.review_result_path);
    let review = review_delta::verify_chain_head(
        repository,
        &manifest_path,
        &request.manifest_hash,
        &mut BTreeSet::new(),
        0,
    )?;
    let receipt_bytes = bounded_file::read_regular(
        &receipt_path,
        CORRECTION_REQUEST_LIMIT,
        "review delivery receipt",
    )?;
    require_exact_hash(
        &receipt_bytes,
        &request.delivery_receipt_hash,
        "review delivery receipt",
    )?;
    let route = review_egress::verify_any_completed_delivery(repository, &receipt_path, &review)?;
    let result_bytes = bounded_file::read_regular(
        &result_path,
        REVIEW_RESULT_LIMIT,
        "structured review result",
    )?;
    require_exact_hash(
        &result_bytes,
        &request.review_result_hash,
        "structured review result",
    )?;
    let inspected = inspect(&result_bytes, &review.review_lenses)?;
    let review_id_drift = inspected.review_id != review.review_id;
    let candidate_drift = inspected.candidate_commit != review.candidate_commit;
    if !review_id_drift && !candidate_drift {
        return Err("structured review result already has the exact identity envelope".to_owned());
    }
    let (route, session_id) = match route {
        review_egress::VerifiedDeliveryRoute::Managed {
            provider,
            model,
            session_id,
        } => (format!("managed/{provider}/{model}"), session_id),
        review_egress::VerifiedDeliveryRoute::Delegated { host, session_id } => {
            (format!("delegated/{host}"), session_id)
        },
    };
    let result = CorrectionResult {
        schema: CORRECTION_RESULT_SCHEMA,
        ok: true,
        status: "eligible_identity_envelope_only",
        next_action: "request_exact_same_session_envelope_correction_once",
        provider_requests: 0,
        expected_review_id: review.review_id,
        observed_review_id: inspected.review_id,
        expected_candidate_commit: review.candidate_commit,
        observed_candidate_commit: inspected.candidate_commit,
        session_id,
        route,
        immutable_result_hash: digest(&result_bytes),
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode correction preflight result: {error}"))?
    );
    Ok(())
}

fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(format!(
            "{label} must be a canonical sha256:<64 lowercase hex> identity"
        ))
    }
}

fn require_exact_hash(bytes: &[u8], expected: &str, label: &str) -> Result<(), String> {
    if digest(bytes) == expected {
        Ok(())
    } else {
        Err(format!("{label} hash does not match its frozen bytes"))
    }
}

fn exactly_one(bytes: &[u8], needle: &[u8], label: &str) -> Result<usize, String> {
    let mut matches = bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, value)| (value == needle).then_some(index));
    let first = matches
        .next()
        .ok_or_else(|| format!("review output is missing {label}"))?;
    if matches.next().is_some() {
        return Err(format!("review output contains more than one {label}"));
    }
    Ok(first)
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn compact(value: &str, limit: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value != value.trim() || value.len() > limit || value.contains('\0') {
        Err(format!(
            "structured review {label} must be non-empty, trimmed, and at most {limit} bytes"
        ))
    } else {
        Ok(())
    }
}

fn compact_token(value: &str, limit: usize, label: &str) -> Result<(), String> {
    compact(value, limit, label)?;
    if value
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        Err(format!(
            "structured review {label} must be one compact token"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
