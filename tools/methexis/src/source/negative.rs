//! Durable exact-revision review holds and invalidations.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Eligibility, FreshnessFailure, UnitFreshness, working_tree};
use crate::{
    check::{
        Diagnostic, DiagnosticPhase, Foundation, is_segment, is_semantic_id,
        normalize_record_bytes, parse_yaml, valid_hash,
    },
    model::{KnowledgeUnit, Owner},
};

const PATH: &str = "methexis/negative-records.yaml";
const SCHEMA: &str = "methexis.negative-records/v1alpha1";
const MAX_EVIDENCE_REFERENCE_BYTES: usize = 1024;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
pub(crate) struct NegativeRecords {
    schema: String,
    records: Vec<NegativeRecord>,
}

impl NegativeRecords {
    pub(crate) fn empty() -> Self {
        Self {
            schema: SCHEMA.to_owned(),
            records: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct NegativeRecord {
    knowledge_id: String,
    revision: String,
    condition: NegativeCondition,
    recorded_by: String,
    evidence: NegativeEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum NegativeCondition {
    Invalid,
    Suspect,
}

impl NegativeCondition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Suspect => "suspect",
        }
    }

    const fn eligibility(self) -> Eligibility {
        match self {
            Self::Invalid => Eligibility::Invalid,
            Self::Suspect => Eligibility::Suspect,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct NegativeEvidence {
    code: String,
    reference: String,
}

pub(crate) fn load(repository_root: &Path) -> Result<NegativeRecords, Vec<Diagnostic>> {
    load_detailed(repository_root).map(|(records, _)| records)
}

pub(super) fn load_captured(
    repository_root: &Path,
) -> Result<(NegativeRecords, working_tree::Capture), FreshnessFailure> {
    load_detailed(repository_root).map_err(|diagnostics| {
        let code = evaluation_failure_code(&diagnostics);
        failure_from_diagnostics(code, diagnostics)
    })
}

fn load_detailed(
    repository_root: &Path,
) -> Result<(NegativeRecords, working_tree::Capture), Vec<Diagnostic>> {
    let (bytes, capture) = working_tree::capture_record(repository_root, PATH)
        .map_err(|failure| vec![capture_failure_diagnostic(failure)])?;
    let content = normalize_record_bytes(&bytes, PATH)?;
    let records = parse_yaml::<NegativeRecords>(&content, PATH, 0)?;
    let diagnostics = validate_local(&records);
    if diagnostics.is_empty() {
        Ok((records, capture))
    } else {
        Err(diagnostics)
    }
}

pub(crate) fn validate_global(
    records: &NegativeRecords,
    units: &[KnowledgeUnit],
    owners: &[Owner],
) -> Vec<Diagnostic> {
    let units = units
        .iter()
        .map(|unit| unit.metadata.id.as_str())
        .collect::<BTreeSet<_>>();
    let owners = owners
        .iter()
        .map(|owner| owner.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    for record in &records.records {
        if !units.contains(record.knowledge_id.as_str()) {
            diagnostics.push(global_diagnostic(
                "unknown_negative_record_knowledge",
                format!(
                    "negative record targets unknown KnowledgeId `{}`",
                    record.knowledge_id
                ),
                vec![record.knowledge_id.clone()],
            ));
        }
        if !owners.contains(record.recorded_by.as_str()) {
            diagnostics.push(global_diagnostic(
                "unknown_negative_record_owner",
                format!(
                    "negative record OwnerId `{}` does not exist",
                    record.recorded_by
                ),
                vec![record.knowledge_id.clone()],
            ));
        }
    }
    diagnostics
}

pub(super) fn validate_for_evaluation(
    records: &NegativeRecords,
    foundation: &Foundation,
) -> Result<(), FreshnessFailure> {
    let diagnostics = validate_global(records, &foundation.units, &foundation.owners);
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(failure_from_diagnostics(
            "negative_records_invalid",
            diagnostics,
        ))
    }
}

pub(super) fn apply(
    trusted: &NegativeRecords,
    working: &NegativeRecords,
    trusted_units: &BTreeMap<&str, &KnowledgeUnit>,
    selected: &BTreeSet<String>,
    states: &mut BTreeMap<String, UnitFreshness>,
) {
    let mut inputs = BTreeMap::<NegativeRecord, BTreeSet<&'static str>>::new();
    for record in &trusted.records {
        inputs.entry(record.clone()).or_default().insert("trusted");
    }
    for record in &working.records {
        inputs.entry(record.clone()).or_default().insert("working");
    }

    for (record, origins) in inputs {
        if !selected.contains(&record.knowledge_id)
            || trusted_units
                .get(record.knowledge_id.as_str())
                .is_none_or(|unit| unit.revision != record.revision)
        {
            continue;
        }
        let state = states
            .get_mut(&record.knowledge_id)
            .expect("every selected KnowledgeId has a guard state");
        state.eligibility = state.eligibility.max(record.condition.eligibility());
        let origin = origins.into_iter().collect::<Vec<_>>().join("+");
        state.evidence.push(format!(
            "negative_record:{origin}:{}:{}",
            record.condition.as_str(),
            record_id(&record)
        ));
        state.evidence.sort();
        state.evidence.dedup();
    }
}

fn validate_local(records: &NegativeRecords) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if records.schema != SCHEMA {
        diagnostics.push(diagnostic(
            "unsupported_negative_records_schema",
            format!("expected negative-record schema `{SCHEMA}`"),
            Vec::new(),
        ));
    }
    let mut seen = BTreeSet::new();
    for record in &records.records {
        let affected = vec![record.knowledge_id.clone()];
        if !is_semantic_id(&record.knowledge_id) {
            diagnostics.push(diagnostic(
                "invalid_negative_record_knowledge_id",
                "negative records require a lowercase semantic KnowledgeId".to_owned(),
                affected.clone(),
            ));
        }
        if !valid_hash(&record.revision) {
            diagnostics.push(diagnostic(
                "invalid_negative_record_revision",
                "negative records require an exact lowercase SHA-256 RevisionId".to_owned(),
                affected.clone(),
            ));
        }
        if !is_segment(&record.recorded_by) {
            diagnostics.push(diagnostic(
                "invalid_negative_record_owner",
                "negative records require a lowercase OwnerId segment".to_owned(),
                affected.clone(),
            ));
        }
        if !is_semantic_id(&record.evidence.code) {
            diagnostics.push(diagnostic(
                "invalid_negative_record_evidence_code",
                "negative-record evidence code must use lowercase semantic segments".to_owned(),
                affected.clone(),
            ));
        }
        if record.evidence.reference.is_empty()
            || record.evidence.reference.len() > MAX_EVIDENCE_REFERENCE_BYTES
            || record.evidence.reference.chars().any(char::is_control)
        {
            diagnostics.push(diagnostic(
                "invalid_negative_record_evidence_reference",
                "negative-record evidence reference must be non-empty, bounded, and control-free"
                    .to_owned(),
                affected.clone(),
            ));
        }
        if !seen.insert(record.clone()) {
            diagnostics.push(diagnostic(
                "duplicate_negative_record",
                "negative-record entries must be unique".to_owned(),
                affected,
            ));
        }
    }
    if records.records.windows(2).any(|pair| pair[0] >= pair[1]) {
        diagnostics.push(diagnostic(
            "noncanonical_negative_record_order",
            "negative-record entries must be strictly sorted by their closed record fields"
                .to_owned(),
            records
                .records
                .iter()
                .map(|record| record.knowledge_id.clone())
                .collect(),
        ));
    }
    diagnostics
}

fn record_id(record: &NegativeRecord) -> String {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, b"domain", b"methexis.negative-record/v1alpha1");
    hash_part(&mut hasher, b"knowledge_id", record.knowledge_id.as_bytes());
    hash_part(&mut hasher, b"revision", record.revision.as_bytes());
    hash_part(
        &mut hasher,
        b"condition",
        record.condition.as_str().as_bytes(),
    );
    hash_part(&mut hasher, b"recorded_by", record.recorded_by.as_bytes());
    hash_part(
        &mut hasher,
        b"evidence_code",
        record.evidence.code.as_bytes(),
    );
    hash_part(
        &mut hasher,
        b"evidence_reference",
        record.evidence.reference.as_bytes(),
    );
    let mut output = String::from("sha256:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in hasher.finalize() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hash_part(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn failure_from_diagnostics(code: &'static str, diagnostics: Vec<Diagnostic>) -> FreshnessFailure {
    FreshnessFailure {
        code,
        message: diagnostics.first().map_or_else(
            || "negative-record input is invalid".to_owned(),
            |diagnostic| diagnostic.message.clone(),
        ),
        affected_ids: diagnostics
            .into_iter()
            .flat_map(|diagnostic| diagnostic.affected_ids)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn evaluation_failure_code(diagnostics: &[Diagnostic]) -> &'static str {
    match diagnostics
        .first()
        .map(|diagnostic| diagnostic.code.as_str())
    {
        Some("negative_records_changed_during_validation") => {
            "negative_records_changed_during_validation"
        },
        Some("negative_records_unavailable") => "negative_records_unavailable",
        _ => "negative_records_invalid",
    }
}

fn capture_failure_diagnostic(failure: FreshnessFailure) -> Diagnostic {
    if failure.code == "source_changed_during_validation" {
        diagnostic(
            "negative_records_changed_during_validation",
            format!(
                "negative-record input changed during capture: {}",
                failure.message
            ),
            Vec::new(),
        )
    } else {
        diagnostic(
            "negative_records_unavailable",
            format!(
                "the tracked negative-record manifest is missing, unreadable, or unsafe: {}",
                failure.message
            ),
            Vec::new(),
        )
    }
}

fn diagnostic(code: &str, message: String, affected_ids: Vec<String>) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Local,
        path: PATH.to_owned(),
        code: code.to_owned(),
        message,
        line: None,
        column: None,
        affected_ids,
    }
}

fn global_diagnostic(code: &str, message: String, affected_ids: Vec<String>) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path: PATH.to_owned(),
        code: code.to_owned(),
        message,
        line: None,
        column: None,
        affected_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 최초 manifest 캡처 중 감지된 identity 변경은 안정적인 missing/unsafe 입력이 아니다.
    // negative-record 전용 동시 변경 코드로 보존해 상위 authority 경계가 retryable로 분류한다.
    #[test]
    fn concurrent_capture_failure_keeps_the_retryable_negative_record_code() {
        let diagnostics = vec![capture_failure_diagnostic(FreshnessFailure {
            code: "source_changed_during_validation",
            message: "changed".to_owned(),
            affected_ids: vec![PATH.to_owned()],
        })];
        let code = evaluation_failure_code(&diagnostics);

        assert_eq!(
            diagnostics[0].code,
            "negative_records_changed_during_validation"
        );
        assert_eq!(code, "negative_records_changed_during_validation");

        let authority = crate::checkpoint::AuthorityFailure::from_source(
            "0123456789abcdef",
            failure_from_diagnostics(code, diagnostics),
        );
        assert!(authority.retryable);
    }
}
