//! Repository-wide Projection and approval proposal validation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use super::{
    ApprovalRecord, ProjectionRecord, ProposalState, ReviewValidation, global_diagnostic,
    local_diagnostic,
    records::{parse_approval, parse_projection},
    relative_path, sort_diagnostics,
};
use crate::check::{Diagnostic, Foundation};

pub(crate) fn validate_records(
    repository_root: &Path,
    foundation: &Foundation,
) -> ReviewValidation {
    let mut diagnostics = Vec::new();
    let projection_paths = optional_files(
        &repository_root.join("methexis/review-projections"),
        "md",
        repository_root,
        &mut diagnostics,
    );
    let approval_paths = optional_files(
        &repository_root.join("methexis/approvals"),
        "yaml",
        repository_root,
        &mut diagnostics,
    );

    let units = foundation
        .units
        .iter()
        .map(|unit| (unit.metadata.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let owners = foundation
        .owners
        .iter()
        .map(|owner| owner.id.as_str())
        .collect::<BTreeSet<_>>();

    let mut projections = BTreeMap::<String, Vec<ProjectionRecord>>::new();
    for path in projection_paths {
        match parse_projection(&path, repository_root) {
            Ok(record) => projections
                .entry(record.metadata.knowledge_id.clone())
                .or_default()
                .push(record),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }

    let mut approvals = BTreeMap::<String, Vec<(ApprovalRecord, PathBuf)>>::new();
    for path in approval_paths {
        match parse_approval(&path, repository_root) {
            Ok(record) => approvals
                .entry(record.knowledge_id.clone())
                .or_default()
                .push((record, path)),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }

    for (id, records) in &projections {
        if records.len() > 1 {
            for record in records {
                diagnostics.push(global_diagnostic(
                    repository_root,
                    &record.path,
                    "duplicate_review_projection",
                    format!("KnowledgeId `{id}` has more than one review Projection"),
                    vec![id.clone()],
                ));
            }
        }
    }
    for (id, records) in &approvals {
        if records.len() > 1 {
            for (_, path) in records {
                diagnostics.push(global_diagnostic(
                    repository_root,
                    path,
                    "duplicate_approval",
                    format!("KnowledgeId `{id}` has more than one approval record"),
                    vec![id.clone()],
                ));
            }
        }
    }

    let unique_projections = projections
        .iter()
        .filter_map(|(id, records)| (records.len() == 1).then_some((id.as_str(), &records[0])))
        .collect::<BTreeMap<_, _>>();
    for (id, projection) in &unique_projections {
        match units.get(id) {
            None => diagnostics.push(global_diagnostic(
                repository_root,
                &projection.path,
                "unknown_projection_knowledge",
                format!("review Projection targets unknown KnowledgeId `{id}`"),
                vec![(*id).to_owned()],
            )),
            Some(unit) if projection.metadata.revision != unit.revision => {
                diagnostics.push(global_diagnostic(
                    repository_root,
                    &projection.path,
                    "stale_review_projection",
                    format!(
                        "Projection revision `{}` does not match current revision `{}`",
                        projection.metadata.revision, unit.revision
                    ),
                    vec![(*id).to_owned()],
                ));
            },
            Some(_) => {},
        }
    }

    let mut states = foundation
        .units
        .iter()
        .map(|unit| {
            let state = if unique_projections.contains_key(unit.metadata.id.as_str()) {
                ProposalState {
                    evidence: "projection_ready",
                    reason: Some("missing_approval"),
                }
            } else {
                ProposalState {
                    evidence: "missing",
                    reason: Some("missing_approval"),
                }
            };
            (unit.metadata.id.clone(), state)
        })
        .collect::<BTreeMap<_, _>>();

    for (id, records) in approvals {
        if records.len() != 1 {
            continue;
        }
        let (approval, path) = &records[0];
        let Some(unit) = units.get(id.as_str()) else {
            diagnostics.push(global_diagnostic(
                repository_root,
                path,
                "unknown_approval_knowledge",
                format!("approval targets unknown KnowledgeId `{id}`"),
                vec![id],
            ));
            continue;
        };
        if !owners.contains(approval.reviewer.as_str()) {
            diagnostics.push(global_diagnostic(
                repository_root,
                path,
                "unknown_reviewer",
                format!("reviewer OwnerId `{}` does not exist", approval.reviewer),
                vec![id],
            ));
            continue;
        }
        if approval.revision != unit.revision {
            states.insert(
                id,
                ProposalState {
                    evidence: "stale_proposal",
                    reason: Some("revision_mismatch"),
                },
            );
            continue;
        }

        let Some(projection) = unique_projections.get(id.as_str()) else {
            diagnostics.push(global_diagnostic(
                repository_root,
                path,
                "missing_approval_projection",
                "current approval has no matching review Projection".to_owned(),
                vec![id],
            ));
            continue;
        };
        if approval.projection_hash != projection.hash
            || approval.projection_profile != projection.metadata.profile
            || approval.projection_compiler != projection.metadata.compiler
        {
            diagnostics.push(global_diagnostic(
                repository_root,
                path,
                "approval_projection_mismatch",
                "approval evidence does not match the exact tracked Projection".to_owned(),
                vec![id],
            ));
            continue;
        }
        states.insert(
            id,
            ProposalState {
                evidence: "matching_proposal",
                reason: None,
            },
        );
    }

    sort_diagnostics(&mut diagnostics);
    ReviewValidation {
        states,
        diagnostics,
    }
}

fn optional_files(
    root: &Path,
    extension: &str,
    repository_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PathBuf> {
    match fs::symlink_metadata(root) {
        Ok(_) => {},
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            diagnostics.push(local_diagnostic(
                relative_path(repository_root, root),
                "review_path_unreadable",
                error.to_string(),
                Vec::new(),
            ));
            return Vec::new();
        },
    }
    let mut files = Vec::new();
    collect_optional_files(root, extension, repository_root, diagnostics, &mut files);
    files.sort();
    files
}

fn collect_optional_files(
    root: &Path,
    extension: &str,
    repository_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
    files: &mut Vec<PathBuf>,
) {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) => {
            diagnostics.push(local_diagnostic(
                relative_path(repository_root, root),
                "review_path_unreadable",
                error.to_string(),
                Vec::new(),
            ));
            return;
        },
    };
    if metadata.file_type().is_symlink() {
        diagnostics.push(local_diagnostic(
            relative_path(repository_root, root),
            "symlink_forbidden",
            "review records and directories must not be symlinks".to_owned(),
            Vec::new(),
        ));
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(local_diagnostic(
                relative_path(repository_root, root),
                "review_path_unreadable",
                error.to_string(),
                Vec::new(),
            ));
            return;
        },
    };
    let mut entries = match entries.collect::<Result<Vec<_>, _>>() {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(local_diagnostic(
                relative_path(repository_root, root),
                "review_path_unreadable",
                error.to_string(),
                Vec::new(),
            ));
            return;
        },
    };
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                diagnostics.push(local_diagnostic(
                    relative_path(repository_root, &path),
                    "review_path_unreadable",
                    error.to_string(),
                    Vec::new(),
                ));
                continue;
            },
        };
        if file_type.is_symlink() {
            diagnostics.push(local_diagnostic(
                relative_path(repository_root, &path),
                "symlink_forbidden",
                "review records and directories must not be symlinks".to_owned(),
                Vec::new(),
            ));
        } else if file_type.is_dir() {
            collect_optional_files(&path, extension, repository_root, diagnostics, files);
        } else if file_type.is_file() && path.extension() == Some(extension.as_ref()) {
            files.push(path);
        }
    }
}
