use std::collections::{BTreeMap, BTreeSet};

use serde_json::from_slice;

use super::{
    SCHEMA,
    bounds::{
        END, MAX_FINDINGS, MAX_SUMMARY_BYTES, START, compact, compact_token, exactly_one,
        trim_ascii,
    },
    model::{InspectedResult, ResultDocument, VerifiedResult},
};

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

pub(super) fn inspect(
    response: &[u8],
    expected_lenses: &[String],
) -> Result<InspectedResult, String> {
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
    let document: ResultDocument = from_slice(trim_ascii(&response[json_start..json_end]))
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
