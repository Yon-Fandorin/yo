//! Compact, reproducible success output for Checkpoint transitions.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    CheckpointRecord, OperationFailure,
    git::TrustedSnapshot,
    hash_bytes,
    records::{read_active, read_checkpoint},
};

#[derive(Clone, Debug, Serialize)]
pub(super) struct CheckpointDelta {
    baseline: Option<BaselineCheckpoint>,
    candidate: CandidateCheckpoint,
    unit_changes: Vec<UnitChange>,
    root_changes: Vec<RootChange>,
    candidate_unit_count: usize,
    unchanged_unit_count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct BaselineCheckpoint {
    checkpoint_id: String,
    checkpoint_hash: String,
}

#[derive(Clone, Debug, Serialize)]
struct CandidateCheckpoint {
    checkpoint_id: String,
    checkpoint_hash: String,
    artifact_path: String,
}

#[derive(Clone, Debug, Serialize)]
struct UnitChange {
    id: String,
    before_revision: Option<String>,
    after_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct RootChange {
    root: String,
    before_present: bool,
    after_present: bool,
}

pub(super) fn summarize(
    snapshot: &TrustedSnapshot,
    candidate: &CheckpointRecord,
    candidate_hash: &str,
    candidate_path: String,
    operation: &'static str,
) -> Result<CheckpointDelta, OperationFailure> {
    let baseline = read_baseline(snapshot, operation)?;
    Ok(compare(
        baseline
            .as_ref()
            .map(|(record, hash)| (record, hash.as_str())),
        candidate,
        candidate_hash,
        candidate_path,
    ))
}

fn compare(
    baseline: Option<(&CheckpointRecord, &str)>,
    candidate: &CheckpointRecord,
    candidate_hash: &str,
    candidate_path: String,
) -> CheckpointDelta {
    let before_units = baseline
        .map(|(record, _)| revisions(record))
        .unwrap_or_default();
    let after_units = revisions(candidate);

    let mut unit_changes = Vec::new();
    let mut unchanged_unit_count = 0usize;
    for id in before_units
        .keys()
        .chain(after_units.keys())
        .collect::<BTreeSet<_>>()
    {
        let before_revision = before_units.get(id);
        let after_revision = after_units.get(id);
        if before_revision == after_revision {
            unchanged_unit_count += 1;
        } else {
            unit_changes.push(UnitChange {
                id: (*id).clone(),
                before_revision: before_revision.cloned(),
                after_revision: after_revision.cloned(),
            });
        }
    }

    let before_roots = baseline
        .map(|(record, _)| record.roots.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let after_roots = candidate.roots.iter().cloned().collect::<BTreeSet<_>>();
    let root_changes = before_roots
        .union(&after_roots)
        .filter_map(|root| {
            let before_present = before_roots.contains(root);
            let after_present = after_roots.contains(root);
            (before_present != after_present).then(|| RootChange {
                root: root.clone(),
                before_present,
                after_present,
            })
        })
        .collect();

    CheckpointDelta {
        baseline: baseline.map(|(record, hash)| BaselineCheckpoint {
            checkpoint_id: record.checkpoint_id.clone(),
            checkpoint_hash: hash.to_owned(),
        }),
        candidate: CandidateCheckpoint {
            checkpoint_id: candidate.checkpoint_id.clone(),
            checkpoint_hash: candidate_hash.to_owned(),
            artifact_path: candidate_path,
        },
        unit_changes,
        root_changes,
        candidate_unit_count: candidate.units.len(),
        unchanged_unit_count,
    }
}

fn read_baseline(
    snapshot: &TrustedSnapshot,
    operation: &'static str,
) -> Result<Option<(CheckpointRecord, String)>, OperationFailure> {
    let active_path = snapshot.root.join("methexis/active-checkpoint.yaml");
    if !active_path.exists() {
        return Ok(None);
    }
    let (active, _) = read_active(&active_path, operation)?;
    let checkpoint_name = active
        .checkpoint_id
        .strip_prefix("sha256:")
        .expect("validated active CheckpointId has a sha256 prefix");
    let checkpoint_path = snapshot
        .root
        .join("methexis/checkpoints")
        .join(format!("{checkpoint_name}.yaml"));
    let (checkpoint, bytes) = read_checkpoint(&checkpoint_path, operation).map_err(|failure| {
        failure.with_trusted_evidence(snapshot.commit.clone(), active.checkpoint_id.clone())
    })?;
    let checkpoint_hash = hash_bytes(&bytes);
    if checkpoint.checkpoint_id != active.checkpoint_id
        || checkpoint_hash != active.checkpoint_hash
        || checkpoint.trusted_commit != active.trusted_commit
    {
        return Err(OperationFailure::new(
            operation,
            Some(snapshot.commit.clone()),
            "active_checkpoint_lineage_mismatch",
            "active record does not identify its exact immutable Checkpoint",
            vec![active.checkpoint_id],
            "repair the active Checkpoint lineage through repository review",
        ));
    }
    Ok(Some((checkpoint, checkpoint_hash)))
}

fn revisions(record: &CheckpointRecord) -> BTreeMap<String, String> {
    record
        .units
        .iter()
        .map(|unit| (unit.id.clone(), unit.revision.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::checkpoint::CheckpointUnit;

    // 순서가 뒤섞인 입력에서도 revision 교체·추가·삭제와 root-only 변화를 정렬하고 unchanged를
    // 세어야 한다.
    #[test]
    fn comparison_sorts_changes_and_distinguishes_revisions_roots_and_counts() {
        let baseline = record(
            &["root.same", "root.only-before"],
            &[
                ("z.removed", "rev-z"),
                ("b.changed", "rev-b1"),
                ("a.same", "rev-a"),
            ],
        );
        let candidate = record(
            &["root.only-after", "root.same"],
            &[
                ("c.added", "rev-c"),
                ("a.same", "rev-a"),
                ("b.changed", "rev-b2"),
            ],
        );

        let value = serde_json::to_value(compare(
            Some((&baseline, "baseline-hash")),
            &candidate,
            "candidate-hash",
            "candidate-path".to_owned(),
        ))
        .unwrap();

        assert_eq!(value["candidate_unit_count"], 3);
        assert_eq!(value["unchanged_unit_count"], 1);
        assert_eq!(
            value["unit_changes"],
            json!([
                {"id": "b.changed", "before_revision": "rev-b1", "after_revision": "rev-b2"},
                {"id": "c.added", "before_revision": null, "after_revision": "rev-c"},
                {"id": "z.removed", "before_revision": "rev-z", "after_revision": null}
            ])
        );
        assert_eq!(
            value["root_changes"],
            json!([
                {"root": "root.only-after", "before_present": false, "after_present": true},
                {"root": "root.only-before", "before_present": true, "after_present": false}
            ])
        );
    }

    fn record(roots: &[&str], units: &[(&str, &str)]) -> CheckpointRecord {
        CheckpointRecord {
            schema: "test".to_owned(),
            checkpoint_id: "checkpoint".to_owned(),
            trusted_commit: "commit".to_owned(),
            source_status: "not_evaluated".to_owned(),
            roots: roots.iter().map(|root| (*root).to_owned()).collect(),
            units: units
                .iter()
                .map(|(id, revision)| CheckpointUnit {
                    id: (*id).to_owned(),
                    revision: (*revision).to_owned(),
                    reasons: vec!["test".to_owned()],
                })
                .collect(),
        }
    }
}
