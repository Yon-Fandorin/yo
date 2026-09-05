//! Corpus-wide identifier, relation, and cycle validation.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    Diagnostic, cycles,
    diagnostic::{display_path, global_diagnostic},
};
use crate::model::{KnowledgeUnit, Owner, Relations, Source, UnitsById};

pub(super) fn validate_global(
    units: &[KnowledgeUnit],
    owners: &[Owner],
    sources: &[Source],
    negative_records: &crate::source::NegativeRecords,
    repository_root: &Path,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut units_by_id = UnitsById::new();
    for unit in units {
        units_by_id
            .entry(unit.metadata.id.clone())
            .or_default()
            .push(unit.clone());
    }

    let mut owners_by_id = BTreeMap::<String, Vec<&Owner>>::new();
    for owner in owners {
        owners_by_id
            .entry(owner.id.clone())
            .or_default()
            .push(owner);
    }
    let mut sources_by_id = BTreeMap::<String, Vec<&Source>>::new();
    for source in sources {
        sources_by_id
            .entry(source.record.id.clone())
            .or_default()
            .push(source);
    }

    if units.is_empty() {
        diagnostics.push(global_diagnostic(
            "methexis/knowledge".to_owned(),
            "empty_corpus",
            "Draft corpus must contain at least one KnowledgeUnit".to_owned(),
            Vec::new(),
        ));
    }

    for (id, duplicates) in &units_by_id {
        if duplicates.len() > 1 {
            for unit in duplicates {
                diagnostics.push(global_diagnostic(
                    display_path(&unit.path, repository_root),
                    "duplicate_knowledge_id",
                    format!("KnowledgeId `{id}` appears in more than one file"),
                    vec![id.clone()],
                ));
            }
        }
    }
    for (id, duplicates) in &owners_by_id {
        if duplicates.len() > 1 {
            for owner in duplicates {
                diagnostics.push(global_diagnostic(
                    display_path(&owner.path, repository_root),
                    "duplicate_owner_id",
                    format!("OwnerId `{id}` appears in more than one file"),
                    vec![id.clone()],
                ));
            }
        }
    }
    for (id, duplicates) in &sources_by_id {
        if duplicates.len() > 1 {
            for source in duplicates {
                diagnostics.push(global_diagnostic(
                    display_path(&source.path, repository_root),
                    "duplicate_source_id",
                    format!("SourceId `{id}` appears in more than one file"),
                    vec![id.clone()],
                ));
            }
        }
    }

    let known_ids = units_by_id.keys().cloned().collect::<BTreeSet<_>>();
    for unit in units {
        if !owners_by_id.contains_key(&unit.metadata.owner) {
            diagnostics.push(global_diagnostic(
                display_path(&unit.path, repository_root),
                "missing_owner",
                format!("OwnerId `{}` has no owner record", unit.metadata.owner),
                vec![unit.metadata.id.clone()],
            ));
        }
        for source in &unit.metadata.sources {
            if !sources_by_id.contains_key(&source.id) {
                diagnostics.push(global_diagnostic(
                    display_path(&unit.path, repository_root),
                    "missing_source_record",
                    format!("SourceId `{}` has no Source record", source.id),
                    vec![unit.metadata.id.clone(), source.id.clone()],
                ));
            }
        }
        for (relation, targets) in [
            ("depends_on", unit.metadata.relations.depends_on.as_slice()),
            (
                "constrained_by",
                unit.metadata.relations.constrained_by.as_slice(),
            ),
            ("supersedes", unit.metadata.relations.supersedes.as_slice()),
        ] {
            for target in targets {
                if !known_ids.contains(target) {
                    diagnostics.push(global_diagnostic(
                        display_path(&unit.path, repository_root),
                        "missing_relation_target",
                        format!("relation `{relation}` targets missing KnowledgeId `{target}`"),
                        vec![unit.metadata.id.clone(), target.clone()],
                    ));
                }
            }
        }
    }

    let unique_units = units_by_id
        .into_iter()
        .filter_map(|(id, mut entries)| {
            let unit = entries.pop()?;
            entries.is_empty().then_some((id, unit))
        })
        .collect::<BTreeMap<_, _>>();
    diagnostics.extend(cycle_diagnostics(
        &unique_units,
        repository_root,
        "required_relation_cycle",
        |relations| relations.required_targets().cloned().collect::<Vec<_>>(),
    ));
    diagnostics.extend(cycle_diagnostics(
        &unique_units,
        repository_root,
        "supersedes_cycle",
        |relations| relations.supersedes.clone(),
    ));
    diagnostics.extend(crate::source::negative::validate_global(
        negative_records,
        units,
        owners,
    ));
    diagnostics
}

fn cycle_diagnostics(
    units: &BTreeMap<String, KnowledgeUnit>,
    repository_root: &Path,
    code: &str,
    edges: impl Fn(&Relations) -> Vec<String>,
) -> Vec<Diagnostic> {
    cycles::find_cycles(units, edges)
        .into_iter()
        .map(|cycle| {
            let source = cycle.first().and_then(|id| units.get(id)).map_or_else(
                || "methexis/knowledge".to_owned(),
                |unit| display_path(&unit.path, repository_root),
            );
            global_diagnostic(
                source,
                code,
                format!("cycle detected: {}", cycle.join(" -> ")),
                cycle,
            )
        })
        .collect()
}
