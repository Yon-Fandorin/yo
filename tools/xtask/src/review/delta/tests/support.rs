use super::super::capture::captured;
use crate::{
    review::{
        delta::model::{
            Disposition, FindingDisposition, PRIOR_FINDINGS_SCHEMA, PriorFinding, PriorFindings,
        },
        packet::{self as review_packet, VerifiedReview},
    },
    review_protocol::Captured,
};

pub(super) fn hash(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}
pub(super) fn commit(byte: u8) -> String {
    format!("{byte:02x}").repeat(20)
}

pub(super) fn prior(evidence: Vec<review_packet::VerifiedEvidence>) -> VerifiedReview {
    VerifiedReview {
        review_id: hash(1),
        manifest_path: "manifest.json".to_owned(),
        manifest_hash: hash(2),
        packet_path: "packet.md".to_owned(),
        packet_hash: hash(3),
        base_commit: commit(1),
        candidate_commit: commit(2),
        trusted_commit: commit(1),
        slice_contract_path: "slice-contract.json".to_owned(),
        slice_contract_hash: hash(4),
        validation_evidence: evidence,
        review_lenses: vec!["fresh-context".to_owned()],
        review_questions: vec!["Are the findings resolved?".to_owned()],
    }
}

pub(super) fn finding(id: &str) -> FindingDisposition {
    FindingDisposition {
        finding_id: id.to_owned(),
        disposition: Disposition::Resolved,
        summary: "The replacement candidate covers this case.".to_owned(),
    }
}

pub(super) fn prior_findings(ids: &[&str]) -> Captured {
    let value = PriorFindings {
        schema: PRIOR_FINDINGS_SCHEMA.to_owned(),
        review_id: hash(1),
        candidate_commit: commit(2),
        findings: ids
            .iter()
            .map(|id| PriorFinding {
                finding_id: (*id).to_owned(),
                summary: format!("Finding {id}"),
            })
            .collect(),
    };
    captured(
        "prior-findings.json".to_owned(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap()
}
