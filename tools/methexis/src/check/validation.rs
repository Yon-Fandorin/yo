use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    Diagnostic, DiagnosticPhase, KNOWLEDGE_SCHEMA,
    body::{BodyLine, classify_body_lines},
    cycles,
};
use crate::{
    model::{KnowledgeKind, KnowledgeMetadata, KnowledgeUnit, Owner, Relations, Source, UnitsById},
    source,
};

pub(super) fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            left.phase,
            &left.path,
            &left.code,
            left.line,
            left.column,
            &left.message,
            &left.affected_ids,
        )
            .cmp(&(
                right.phase,
                &right.path,
                &right.code,
                right.line,
                right.column,
                &right.message,
                &right.affected_ids,
            ))
    });
}

pub(crate) fn validate_metadata(
    metadata: &KnowledgeMetadata,
    body: &str,
    body_start_line: u64,
    path: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let affected = vec![metadata.id.clone()];

    if metadata.schema != KNOWLEDGE_SCHEMA {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "unsupported_schema",
            format!("expected knowledge schema `{KNOWLEDGE_SCHEMA}`"),
            None,
            None,
            affected.clone(),
        ));
    }
    if !is_semantic_id(&metadata.id) {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "invalid_knowledge_id",
            "KnowledgeId must use lowercase dot-separated semantic segments".to_owned(),
            None,
            None,
            affected.clone(),
        ));
    }
    if !is_segment(&metadata.owner) {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "invalid_owner_id",
            "OwnerId must be one lowercase semantic segment".to_owned(),
            None,
            None,
            affected.clone(),
        ));
    }
    if metadata.sources.is_empty() {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "missing_source",
            "at least one SourceId is required".to_owned(),
            None,
            None,
            affected.clone(),
        ));
    }

    let source_ids = metadata
        .sources
        .iter()
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    validate_unique_ids(&source_ids, "source", path, &affected, &mut diagnostics);
    for source in &metadata.sources {
        if !is_semantic_id(&source.id) {
            diagnostics.push(local_diagnostic(
                path.to_owned(),
                "invalid_source_id",
                format!("invalid SourceId `{}`", source.id),
                None,
                None,
                affected.clone(),
            ));
        }
        if !valid_hash(&source.revision) {
            diagnostics.push(local_diagnostic(
                path.to_owned(),
                "invalid_source_revision",
                format!(
                    "Source `{}` must pin a lowercase SHA-256 SourceRevision",
                    source.id
                ),
                None,
                None,
                affected.clone(),
            ));
        }
    }

    for (relation, targets) in metadata.relations.typed() {
        validate_unique_ids(targets, relation, path, &affected, &mut diagnostics);
        for target in targets {
            if target.is_empty() {
                diagnostics.push(local_diagnostic(
                    path.to_owned(),
                    "empty_relation_target",
                    format!("relation `{relation}` contains an empty target"),
                    None,
                    None,
                    affected.clone(),
                ));
            } else if matches!(relation, "depends_on" | "constrained_by" | "supersedes")
                && !is_semantic_id(target)
            {
                diagnostics.push(local_diagnostic(
                    path.to_owned(),
                    "invalid_relation_target",
                    format!("relation `{relation}` has invalid KnowledgeId `{target}`"),
                    None,
                    None,
                    affected.clone(),
                ));
            }
        }
    }

    let body_lines = classify_body_lines(body);
    if let Some((index, _)) = body_lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.forbidden_html)
    {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "raw_html_forbidden",
            "canonical Markdown bodies must not contain raw HTML blocks".to_owned(),
            Some(body_start_line + index as u64),
            Some(1),
            affected.clone(),
        ));
    }

    require_body_section(
        &body_lines,
        "Statement",
        body_start_line,
        path,
        &affected,
        &mut diagnostics,
    );
    if metadata.kind == KnowledgeKind::Decision {
        require_body_section(
            &body_lines,
            "Rationale",
            body_start_line,
            path,
            &affected,
            &mut diagnostics,
        );
    }
    if metadata.kind == KnowledgeKind::Procedure {
        require_body_section(
            &body_lines,
            "Steps",
            body_start_line,
            path,
            &affected,
            &mut diagnostics,
        );
        require_body_section(
            &body_lines,
            "Completion Criteria",
            body_start_line,
            path,
            &affected,
            &mut diagnostics,
        );
    }

    diagnostics
}

fn validate_unique_ids(
    values: &[String],
    field: &str,
    path: &str,
    affected: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            diagnostics.push(local_diagnostic(
                path.to_owned(),
                "duplicate_reference",
                format!("`{field}` contains duplicate target `{value}`"),
                None,
                None,
                affected.to_vec(),
            ));
        }
    }
}

fn require_body_section(
    lines: &[BodyLine<'_>],
    name: &str,
    body_start_line: u64,
    path: &str,
    affected: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let heading = format!("## {name}");
    let positions = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.heading == Some(heading.as_str())).then_some(index))
        .collect::<Vec<_>>();

    match positions.as_slice() {
        [] => diagnostics.push(local_diagnostic(
            path.to_owned(),
            "missing_body_section",
            format!("canonical body requires section `{heading}`"),
            None,
            None,
            affected.to_vec(),
        )),
        [position] => {
            let content_exists = lines[position + 1..]
                .iter()
                .take_while(|line| line.heading.is_none())
                .any(|line| line.has_content);
            if !content_exists {
                diagnostics.push(local_diagnostic(
                    path.to_owned(),
                    "empty_body_section",
                    format!("canonical body section `{heading}` must not be empty"),
                    Some(body_start_line + *position as u64),
                    Some(1),
                    affected.to_vec(),
                ));
            }
        },
        _ => diagnostics.push(local_diagnostic(
            path.to_owned(),
            "duplicate_body_section",
            format!("canonical body section `{heading}` appears more than once"),
            None,
            None,
            affected.to_vec(),
        )),
    }
}

pub(super) fn validate_global(
    units: &[KnowledgeUnit],
    owners: &[Owner],
    sources: &[Source],
    negative_records: &source::NegativeRecords,
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
    diagnostics.extend(source::negative::validate_global(
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

pub(crate) fn is_semantic_id(id: &str) -> bool {
    !id.is_empty() && id.split('.').all(is_segment)
}

pub(crate) fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn is_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    matches!(bytes.first(), Some(b'a'..=b'z'))
        && matches!(bytes.last(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && !segment.contains("--")
}

pub(super) fn display_path(path: &Path, repository_root: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn local_diagnostic(
    path: String,
    code: impl Into<String>,
    message: String,
    line: Option<u64>,
    column: Option<u64>,
    affected_ids: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Local,
        path,
        code: code.into(),
        message,
        line,
        column,
        affected_ids,
    }
}

pub(super) fn global_diagnostic(
    path: String,
    code: impl Into<String>,
    message: String,
    affected_ids: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path,
        code: code.into(),
        message,
        line: None,
        column: None,
        affected_ids,
    }
}
