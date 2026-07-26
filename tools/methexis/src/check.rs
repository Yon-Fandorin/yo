use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{
    KNOWLEDGE_SCHEMA, KnowledgeKind, KnowledgeMetadata, KnowledgeUnit, OWNER_SCHEMA, Owner,
    OwnerRecord, Relations, Source, UnitsById,
};

const CHECK_SCHEMA: &str = "methexis.check/v1alpha1";
const REVISION_DOMAIN: &[u8] = b"methexis.knowledge-revision/v1alpha1";
const SNAPSHOT_DOMAIN: &[u8] = b"methexis.knowledge-snapshot/v1alpha1";
const MAX_RECORD_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPhase {
    Local,
    Global,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    pub phase: DiagnosticPhase,
    pub path: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u64>,
    pub affected_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnitRevision {
    pub id: String,
    pub revision: String,
    pub path: String,
    pub effective_approval: &'static str,
    pub approval_evidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_reason: Option<&'static str>,
    pub eligibility: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub eligibility_evidence: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckReport {
    pub schema: &'static str,
    pub ok: bool,
    pub authority: &'static str,
    pub approval: &'static str,
    pub checkpoint: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_commit: Option<String>,
    pub snapshot_revision: Option<String>,
    pub affected_ids: Vec<String>,
    pub units: Vec<UnitRevision>,
    pub diagnostics: Vec<Diagnostic>,
    pub next_actions: Vec<String>,
}

pub(crate) struct Foundation {
    pub(crate) units: Vec<KnowledgeUnit>,
    pub(crate) owners: Vec<Owner>,
    pub(crate) sources: Vec<Source>,
}

pub(crate) fn check_repository(repository_root: &Path) -> CheckReport {
    let mut foundation = match load_foundation(repository_root) {
        Ok(foundation) => foundation,
        Err(mut diagnostics) => {
            sort_diagnostics(&mut diagnostics);
            return failed_report(diagnostics);
        },
    };
    let review_validation = crate::review::validate_records(repository_root, &foundation);
    if !review_validation.diagnostics.is_empty() {
        return failed_report(review_validation.diagnostics);
    }
    let authority = match crate::checkpoint::evaluate(repository_root, Some(&foundation.sources)) {
        Ok(authority) => authority,
        Err(mut failure) => {
            sort_diagnostics(&mut failure.diagnostics);
            return failed_authority_report(failure);
        },
    };

    foundation
        .units
        .sort_by(|left, right| left.metadata.id.cmp(&right.metadata.id));
    let snapshot_revision = snapshot_revision(&foundation.units);
    let unit_revisions = foundation
        .units
        .into_iter()
        .map(|unit| {
            let state = review_validation.states.get(&unit.metadata.id);
            let trusted_approval = authority.as_ref().is_some_and(|authority| {
                authority.approvals.get(&unit.metadata.id) == Some(&unit.revision)
            });
            let active = trusted_approval
                && authority
                    .as_ref()
                    .is_some_and(|authority| authority.active.contains(&unit.metadata.id));
            let freshness = authority
                .as_ref()
                .and_then(|authority| authority.freshness.get(&unit.metadata.id));
            UnitRevision {
                id: unit.metadata.id,
                revision: unit.revision,
                path: display_path(&unit.path, repository_root),
                effective_approval: if trusted_approval {
                    "approved"
                } else {
                    "draft"
                },
                approval_evidence: if trusted_approval {
                    "trusted_approval"
                } else {
                    state.map_or("missing", |state| state.evidence)
                },
                approval_reason: if trusted_approval {
                    None
                } else {
                    state.and_then(|state| state.reason)
                },
                eligibility: if active {
                    "active"
                } else if trusted_approval {
                    freshness.map_or("inactive", |state| state.eligibility.as_str())
                } else if authority.is_some() {
                    "inactive"
                } else {
                    "not_evaluated"
                },
                eligibility_evidence: if !trusted_approval {
                    Vec::new()
                } else {
                    freshness.map_or_else(Vec::new, |state| state.evidence.clone())
                },
            }
        })
        .collect();

    CheckReport {
        schema: CHECK_SCHEMA,
        ok: true,
        authority: "draft",
        approval: if authority.is_some() {
            "trusted_evaluated"
        } else {
            "proposal_evaluated"
        },
        checkpoint: authority
            .as_ref()
            .map_or("not_evaluated", |authority| authority.checkpoint),
        retryable: false,
        trusted_commit: authority
            .as_ref()
            .map(|authority| authority.trusted_commit.clone()),
        snapshot_revision: Some(snapshot_revision),
        affected_ids: Vec::new(),
        units: unit_revisions,
        diagnostics: Vec::new(),
        next_actions: Vec::new(),
    }
}

pub(crate) fn load_foundation(repository_root: &Path) -> Result<Foundation, Vec<Diagnostic>> {
    let corpus_root = repository_root.join("methexis");
    if let Some(diagnostic) = authority_root_diagnostic(&corpus_root, repository_root) {
        return Err(vec![diagnostic]);
    }
    let mut diagnostics = Vec::new();
    let knowledge_paths = collect_files(
        &corpus_root.join("knowledge"),
        "md",
        repository_root,
        &mut diagnostics,
    );
    let owner_paths = collect_files(
        &corpus_root.join("owners"),
        "yaml",
        repository_root,
        &mut diagnostics,
    );
    let sources = match crate::source::load(repository_root) {
        Ok(sources) => sources,
        Err(mut source_diagnostics) => {
            diagnostics.append(&mut source_diagnostics);
            Vec::new()
        },
    };

    let mut units = Vec::new();
    for path in knowledge_paths {
        match parse_knowledge_file(&path, repository_root) {
            Ok(unit) => units.push(unit),
            Err(mut file_diagnostics) => diagnostics.append(&mut file_diagnostics),
        }
    }

    let mut owners = Vec::new();
    for path in owner_paths {
        match parse_owner_file(&path, repository_root) {
            Ok(owner) => owners.push(owner),
            Err(mut file_diagnostics) => diagnostics.append(&mut file_diagnostics),
        }
    }

    sort_diagnostics(&mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut global_diagnostics = validate_global(&units, &owners, &sources, repository_root);
    sort_diagnostics(&mut global_diagnostics);
    if !global_diagnostics.is_empty() {
        return Err(global_diagnostics);
    }

    Ok(Foundation {
        units,
        owners,
        sources,
    })
}

fn authority_root_diagnostic(root: &Path, repository_root: &Path) -> Option<Diagnostic> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => Some(local_diagnostic(
            display_path(root, repository_root),
            "symlink_forbidden",
            "the tracked authority root must not be a symlink".to_owned(),
            None,
            None,
            Vec::new(),
        )),
        Ok(metadata) if metadata.is_dir() => None,
        Ok(_) => Some(local_diagnostic(
            display_path(root, repository_root),
            "corpus_unreadable",
            "the tracked authority root must be a directory".to_owned(),
            None,
            None,
            Vec::new(),
        )),
        Err(error) => Some(local_diagnostic(
            display_path(root, repository_root),
            "corpus_unreadable",
            format!("cannot inspect tracked authority root: {error}"),
            None,
            None,
            Vec::new(),
        )),
    }
}

fn failed_report(diagnostics: Vec<Diagnostic>) -> CheckReport {
    failed_report_with_context(diagnostics, None, false)
}

pub(crate) fn failed_authority_report(failure: crate::checkpoint::AuthorityFailure) -> CheckReport {
    failed_report_with_context(
        failure.diagnostics,
        failure.trusted_commit,
        failure.retryable,
    )
}

fn is_false(value: &bool) -> bool {
    !value
}

fn failed_report_with_context(
    diagnostics: Vec<Diagnostic>,
    trusted_commit: Option<String>,
    retryable: bool,
) -> CheckReport {
    let affected_ids = diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.affected_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    CheckReport {
        schema: CHECK_SCHEMA,
        ok: false,
        authority: "draft",
        approval: "not_evaluated",
        checkpoint: "not_evaluated",
        retryable,
        trusted_commit,
        snapshot_revision: None,
        affected_ids,
        units: Vec::new(),
        diagnostics,
        next_actions: if retryable {
            vec!["retry `methexis check`; no state was published".to_owned()]
        } else {
            vec!["fix the listed diagnostics and rerun `methexis check`".to_owned()]
        },
    }
}

fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
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

pub(crate) fn collect_files(
    root: &Path,
    extension: &str,
    repository_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files_recursive(root, extension, repository_root, diagnostics, &mut files);
    files.sort();
    files
}

fn collect_files_recursive(
    root: &Path,
    extension: &str,
    repository_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
    files: &mut Vec<PathBuf>,
) {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            diagnostics.push(local_diagnostic(
                display_path(root, repository_root),
                "symlink_forbidden",
                "tracked authority directories must not be symlinks".to_owned(),
                None,
                None,
                Vec::new(),
            ));
            return;
        },
        Ok(_) => {},
        Err(error) => {
            diagnostics.push(local_diagnostic(
                display_path(root, repository_root),
                "corpus_unreadable",
                format!("cannot inspect corpus directory: {error}"),
                None,
                None,
                Vec::new(),
            ));
            return;
        },
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(local_diagnostic(
                display_path(root, repository_root),
                "corpus_unreadable",
                format!("cannot read corpus directory: {error}"),
                None,
                None,
                Vec::new(),
            ));
            return;
        },
    };

    let mut entries = match entries.collect::<Result<Vec<_>, _>>() {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(local_diagnostic(
                display_path(root, repository_root),
                "corpus_unreadable",
                format!("cannot enumerate corpus directory: {error}"),
                None,
                None,
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
                    display_path(&path, repository_root),
                    "path_unreadable",
                    format!("cannot inspect corpus path: {error}"),
                    None,
                    None,
                    Vec::new(),
                ));
                continue;
            },
        };

        if file_type.is_symlink() {
            diagnostics.push(local_diagnostic(
                display_path(&path, repository_root),
                "symlink_forbidden",
                "tracked authority records must not be symlinks".to_owned(),
                None,
                None,
                Vec::new(),
            ));
        } else if file_type.is_dir() {
            collect_files_recursive(&path, extension, repository_root, diagnostics, files);
        } else if file_type.is_file() && path.extension() == Some(extension.as_ref()) {
            files.push(path);
        }
    }
}

fn parse_knowledge_file(
    path: &Path,
    repository_root: &Path,
) -> Result<KnowledgeUnit, Vec<Diagnostic>> {
    let display_path = display_path(path, repository_root);
    let content = read_normalized(path, &display_path)?;
    let (frontmatter, body) = split_frontmatter(&content).map_err(|message| {
        vec![local_diagnostic(
            display_path.clone(),
            "invalid_frontmatter",
            message,
            Some(1),
            Some(1),
            Vec::new(),
        )]
    })?;

    let metadata = parse_yaml::<KnowledgeMetadata>(frontmatter, &display_path, 1)?;
    let body_start_line = body_start_line(&content, body);
    let mut diagnostics = validate_metadata(&metadata, body, body_start_line, &display_path);
    if !diagnostics.is_empty() {
        sort_diagnostics(&mut diagnostics);
        return Err(diagnostics);
    }

    let revision = knowledge_revision(&metadata, body);
    Ok(KnowledgeUnit {
        metadata,
        body: body.to_owned(),
        path: path.to_owned(),
        revision,
    })
}

fn body_start_line(content: &str, body: &str) -> u64 {
    let body_offset = content.len() - body.len();
    content[..body_offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u64
        + 1
}

fn parse_owner_file(path: &Path, repository_root: &Path) -> Result<Owner, Vec<Diagnostic>> {
    let display_path = display_path(path, repository_root);
    let content = read_normalized(path, &display_path)?;
    let record = parse_yaml::<OwnerRecord>(&content, &display_path, 0)?;
    let mut diagnostics = Vec::new();

    if record.schema != OWNER_SCHEMA {
        diagnostics.push(local_diagnostic(
            display_path.clone(),
            "unsupported_schema",
            format!("expected owner schema `{OWNER_SCHEMA}`"),
            None,
            None,
            vec![record.id.clone()],
        ));
    }
    if !is_segment(&record.id) {
        diagnostics.push(local_diagnostic(
            display_path,
            "invalid_owner_id",
            "OwnerId must be one lowercase semantic segment".to_owned(),
            None,
            None,
            vec![record.id.clone()],
        ));
    }

    if diagnostics.is_empty() {
        Ok(Owner {
            id: record.id,
            path: path.to_owned(),
        })
    } else {
        Err(diagnostics)
    }
}

pub(crate) fn read_normalized(path: &Path, display_path: &str) -> Result<String, Vec<Diagnostic>> {
    let bytes = fs::read(path).map_err(|error| {
        vec![local_diagnostic(
            display_path.to_owned(),
            "file_unreadable",
            format!("cannot read record: {error}"),
            None,
            None,
            Vec::new(),
        )]
    })?;

    normalize_record_bytes(&bytes, display_path)
}

pub(crate) fn normalize_record_bytes(
    bytes: &[u8],
    display_path: &str,
) -> Result<String, Vec<Diagnostic>> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(vec![local_diagnostic(
            display_path.to_owned(),
            "record_too_large",
            format!(
                "record is {} bytes; the Pilot limit is {MAX_RECORD_BYTES} bytes",
                bytes.len()
            ),
            None,
            None,
            Vec::new(),
        )]);
    }

    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(vec![local_diagnostic(
            display_path.to_owned(),
            "bom_forbidden",
            "UTF-8 BOM is not part of the canonical representation".to_owned(),
            Some(1),
            Some(1),
            Vec::new(),
        )]);
    }

    let content = String::from_utf8(bytes.to_vec()).map_err(|error| {
        vec![local_diagnostic(
            display_path.to_owned(),
            "invalid_utf8",
            format!("record is not valid UTF-8: {error}"),
            None,
            None,
            Vec::new(),
        )]
    })?;

    Ok(normalize_line_endings(content))
}

fn normalize_line_endings(content: String) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let remainder = content
        .strip_prefix("---\n")
        .ok_or_else(|| "knowledge file must start with a `---` delimiter".to_owned())?;
    let (frontmatter, body) = remainder
        .split_once("\n---\n")
        .ok_or_else(|| "knowledge frontmatter must end with a `---` delimiter".to_owned())?;

    if body.trim().is_empty() {
        return Err("canonical Markdown body must not be empty".to_owned());
    }
    Ok((frontmatter, body))
}

pub(crate) fn parse_yaml<T>(yaml: &str, path: &str, line_offset: u64) -> Result<T, Vec<Diagnostic>>
where
    T: serde::de::DeserializeOwned,
{
    serde_norway::from_str(yaml).map_err(|error| {
        let location = error.location();
        vec![local_diagnostic(
            path.to_owned(),
            "invalid_yaml",
            error.to_string(),
            location
                .as_ref()
                .map(|location| location.line() as u64 + line_offset),
            location.as_ref().map(|location| location.column() as u64),
            Vec::new(),
        )]
    })
}

fn validate_metadata(
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

struct BodyLine<'a> {
    heading: Option<&'a str>,
    has_content: bool,
    forbidden_html: bool,
}

fn classify_body_lines(body: &str) -> Vec<BodyLine<'_>> {
    let mut fence = None;
    let mut html_comment = false;
    body.lines()
        .map(|line| {
            let marker = (!html_comment).then(|| fence_marker(line)).flatten();
            let outside_fence = fence.is_none() && marker.is_none();
            let contains_comment_marker = line.contains("<!--") || line.contains("-->");
            let forbidden_html = outside_fence
                && (html_comment || contains_comment_marker || line.trim_start().starts_with('<'));
            let heading = if outside_fence
                && !html_comment
                && !contains_comment_marker
                && line.starts_with("## ")
            {
                Some(line)
            } else {
                None
            };

            match (fence, marker) {
                (None, Some(opening)) => fence = Some(opening),
                (Some((character, minimum)), Some((candidate, length)))
                    if character == candidate
                        && length >= minimum
                        && fence_closing_line(line, character, length) =>
                {
                    fence = None;
                },
                _ => {},
            }

            if outside_fence {
                update_html_comment_state(line, &mut html_comment);
            }

            BodyLine {
                heading,
                has_content: !forbidden_html && !line.trim().is_empty(),
                forbidden_html,
            }
        })
        .collect()
}

pub(crate) fn body_has_forbidden_html(body: &str) -> bool {
    classify_body_lines(body)
        .iter()
        .any(|line| line.forbidden_html)
}

fn update_html_comment_state(mut line: &str, html_comment: &mut bool) {
    loop {
        if *html_comment {
            let Some(end) = line.find("-->") else {
                return;
            };
            *html_comment = false;
            line = &line[end + 3..];
        } else {
            let Some(start) = line.find("<!--") else {
                return;
            };
            *html_comment = true;
            line = &line[start + 4..];
        }
    }
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let candidate = line.as_bytes();
    let indentation = candidate.iter().take_while(|byte| **byte == b' ').count();
    if indentation > 3 {
        return None;
    }
    let marker = *candidate.get(indentation)?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = candidate[indentation..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    (length >= 3).then_some((marker, length))
}

fn fence_closing_line(line: &str, marker: u8, length: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    trimmed
        .as_bytes()
        .get(length..)
        .is_some_and(|remainder| remainder.iter().all(u8::is_ascii_whitespace))
        && trimmed.as_bytes().first() == Some(&marker)
}

fn validate_global(
    units: &[KnowledgeUnit],
    owners: &[Owner],
    sources: &[Source],
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
    diagnostics.extend(validate_cycles(
        &unique_units,
        repository_root,
        "required_relation_cycle",
        |relations| relations.required_targets().cloned().collect::<Vec<_>>(),
    ));
    diagnostics.extend(validate_cycles(
        &unique_units,
        repository_root,
        "supersedes_cycle",
        |relations| relations.supersedes.clone(),
    ));

    diagnostics
}

fn validate_cycles(
    units: &BTreeMap<String, KnowledgeUnit>,
    repository_root: &Path,
    code: &str,
    edges: impl Fn(&Relations) -> Vec<String>,
) -> Vec<Diagnostic> {
    let graph = units
        .iter()
        .map(|(id, unit)| {
            let mut targets = edges(&unit.metadata.relations);
            targets.retain(|target| units.contains_key(target));
            targets.sort();
            (id.clone(), targets)
        })
        .collect::<BTreeMap<_, _>>();
    let mut states = BTreeMap::<String, VisitState>::new();
    let mut stack = Vec::new();
    let mut cycles = BTreeSet::new();

    for id in graph.keys() {
        visit(id, &graph, &mut states, &mut stack, &mut cycles);
    }

    cycles
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Visited,
}

fn visit(
    id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    states: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
    cycles: &mut BTreeSet<Vec<String>>,
) {
    match states.get(id) {
        Some(VisitState::Visited) => return,
        Some(VisitState::Visiting) => {
            if let Some(start) = stack.iter().position(|entry| entry == id) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(id.to_owned());
                cycles.insert(canonical_cycle(cycle));
            }
            return;
        },
        None => {},
    }

    states.insert(id.to_owned(), VisitState::Visiting);
    stack.push(id.to_owned());
    if let Some(targets) = graph.get(id) {
        for target in targets {
            visit(target, graph, states, stack, cycles);
        }
    }
    stack.pop();
    states.insert(id.to_owned(), VisitState::Visited);
}

fn canonical_cycle(mut cycle: Vec<String>) -> Vec<String> {
    cycle.pop();
    if cycle.is_empty() {
        return cycle;
    }
    let start = cycle
        .iter()
        .enumerate()
        .min_by_key(|(_, id)| *id)
        .map_or(0, |(index, _)| index);
    cycle.rotate_left(start);
    cycle.push(cycle[0].clone());
    cycle
}

fn knowledge_revision(metadata: &KnowledgeMetadata, body: &str) -> String {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, b"domain", REVISION_DOMAIN);
    hash_part(&mut hasher, b"schema", metadata.schema.as_bytes());
    hash_part(&mut hasher, b"id", metadata.id.as_bytes());
    hash_part(&mut hasher, b"kind", metadata.kind.as_str().as_bytes());
    hash_part(&mut hasher, b"owner", metadata.owner.as_bytes());
    hash_part(&mut hasher, b"body", body.as_bytes());

    let mut sources = metadata.sources.clone();
    sources.sort();
    for source in sources {
        hash_part(&mut hasher, b"source_id", source.id.as_bytes());
        hash_part(&mut hasher, b"source_revision", source.revision.as_bytes());
    }
    for (relation, targets) in metadata.relations.typed() {
        let mut targets = targets.to_vec();
        targets.sort();
        hash_list(&mut hasher, relation.as_bytes(), &targets);
    }

    tagged_digest(hasher)
}

fn snapshot_revision(units: &[KnowledgeUnit]) -> String {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, b"domain", SNAPSHOT_DOMAIN);
    for unit in units {
        hash_part(&mut hasher, b"id", unit.metadata.id.as_bytes());
        hash_part(&mut hasher, b"revision", unit.revision.as_bytes());
    }
    tagged_digest(hasher)
}

fn tagged_digest(hasher: Sha256) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = hasher.finalize();
    let mut output = String::with_capacity(7 + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn hash_list(hasher: &mut Sha256, label: &[u8], values: &[String]) {
    hash_part(hasher, b"list", label);
    hasher.update((values.len() as u64).to_be_bytes());
    for value in values {
        hash_part(hasher, b"item", value.as_bytes());
    }
}

fn hash_part(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn is_semantic_id(id: &str) -> bool {
    !id.is_empty() && id.split('.').all(is_segment)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    matches!(bytes.first(), Some(b'a'..=b'z'))
        && matches!(bytes.last(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && !segment.contains("--")
}

fn display_path(path: &Path, repository_root: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn local_diagnostic(
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

fn global_diagnostic(
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

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticPhase, knowledge_revision, local_diagnostic, parse_yaml, sort_diagnostics,
        validate_metadata,
    };

    #[test]
    fn semantic_revision_ignores_yaml_order_and_line_endings() {
        let first = "\
---\r\nschema: methexis.knowledge/v1alpha1\r\nid: tui.example\r\nkind: rule\r\nowner: tui-architecture\r\nsources:\r\n  - id: tui.arc-001\r\n    revision: sha256:0000000000000000000000000000000000000000000000000000000000000000\r\nrelations:\r\n  depends_on: []\r\n---\r\n## Statement\r\n\r\nAn example rule.\r\n";
        let second = "\
---\nowner: tui-architecture\nkind: rule\nid: tui.example\nschema: methexis.knowledge/v1alpha1\nrelations:\n  depends_on: []\nsources:\n  - revision: sha256:0000000000000000000000000000000000000000000000000000000000000000\n    id: tui.arc-001\n---\n## Statement\n\nAn example rule.\n";

        let first = parse_for_test(first);
        let second = parse_for_test(second);

        assert_eq!(first, second);
    }

    #[test]
    fn body_change_changes_revision() {
        let metadata = crate::model::KnowledgeMetadata {
            schema: "methexis.knowledge/v1alpha1".to_owned(),
            id: "tui.example".to_owned(),
            kind: crate::model::KnowledgeKind::Rule,
            owner: "tui-architecture".to_owned(),
            sources: vec![source_ref("tui.arc-001")],
            relations: crate::model::Relations::default(),
        };

        assert_ne!(
            knowledge_revision(&metadata, "## Statement\n\nFirst.\n"),
            knowledge_revision(&metadata, "## Statement\n\nSecond.\n"),
        );
    }

    #[test]
    fn semantic_revision_has_a_golden_digest() {
        assert_eq!(
            knowledge_revision(&metadata_for_test(), "## Statement\n\nStable.\n"),
            "sha256:925c20b6fba7467a7d637d7a5ac59cbd183410eb4cc0ade5c20156158f655317",
        );
    }

    #[test]
    fn bare_carriage_returns_normalize_to_lf() {
        assert_eq!(
            super::normalize_line_endings("first\rsecond\r\nthird\n".to_owned()),
            "first\nsecond\nthird\n",
        );
    }

    #[test]
    fn semantic_revision_sorts_sources_and_typed_relations() {
        let mut first = metadata_for_test();
        first.sources = vec![source_ref("tui.source-b"), source_ref("tui.source-a")];
        first.relations.depends_on = vec!["tui.unit-b".to_owned(), "tui.unit-a".to_owned()];
        let mut second = metadata_for_test();
        second.sources = vec![source_ref("tui.source-a"), source_ref("tui.source-b")];
        second.relations.depends_on = vec!["tui.unit-a".to_owned(), "tui.unit-b".to_owned()];

        assert_eq!(
            knowledge_revision(&first, "## Statement\n\nStable.\n"),
            knowledge_revision(&second, "## Statement\n\nStable.\n"),
        );
    }

    #[test]
    fn diagnostic_order_uses_location_before_message() {
        let mut diagnostics = vec![
            local_diagnostic(
                "unit.md".to_owned(),
                "same_code",
                "alphabetically first".to_owned(),
                Some(2),
                Some(1),
                Vec::new(),
            ),
            local_diagnostic(
                "unit.md".to_owned(),
                "same_code",
                "alphabetically last".to_owned(),
                Some(1),
                Some(1),
                Vec::new(),
            ),
        ];

        sort_diagnostics(&mut diagnostics);

        assert_eq!(diagnostics[0].phase, DiagnosticPhase::Local);
        assert_eq!(diagnostics[0].line, Some(1));
    }

    #[test]
    fn retryable_authority_failure_preserves_trusted_commit_and_action() {
        let report = super::failed_authority_report(crate::checkpoint::AuthorityFailure {
            diagnostics: vec![local_diagnostic(
                "methexis/sources".to_owned(),
                "source_changed_during_validation",
                "Source changed".to_owned(),
                None,
                None,
                vec!["tui.example".to_owned()],
            )],
            trusted_commit: Some("0123456789abcdef".to_owned()),
            retryable: true,
        });

        assert!(report.retryable);
        assert_eq!(report.trusted_commit.as_deref(), Some("0123456789abcdef"));
        assert_eq!(
            report.next_actions,
            ["retry `methexis check`; no state was published"]
        );
    }

    #[test]
    fn norway_keeps_yaml_no_as_a_string_for_string_fields() {
        let owner: crate::model::OwnerRecord =
            parse_yaml("schema: methexis.owner/v1alpha1\nid: NO\n", "owner.yaml", 0)
                .expect("NO remains a string at the typed boundary");

        assert_eq!(owner.id, "NO");
    }

    #[test]
    fn norway_rejects_duplicate_mapping_keys_at_the_typed_boundary() {
        let result = parse_yaml::<crate::model::OwnerRecord>(
            "schema: methexis.owner/v1alpha1\nid: first\nid: second\n",
            "owner.yaml",
            0,
        );

        assert!(result.is_err());
    }

    #[test]
    fn norway_rejects_yaml_merge_keys_at_the_typed_boundary() {
        let result = parse_yaml::<crate::model::OwnerRecord>(
            "schema: methexis.owner/v1alpha1\nid: direct\n<<: { id: inherited }\n",
            "owner.yaml",
            0,
        );

        assert!(result.is_err());
    }

    #[test]
    fn headings_inside_fenced_code_do_not_satisfy_body_sections() {
        let metadata = metadata_for_test();
        let diagnostics = validate_metadata(
            &metadata,
            "# Example\n\n```markdown\n## Statement\n\nNot a real section.\n```\n",
            1,
            "unit.md",
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_body_section")
        );
    }

    #[test]
    fn headings_inside_html_comments_make_the_body_invalid() {
        let diagnostics = validate_metadata(
            &metadata_for_test(),
            "# Example\n\nprefix <!--\n## Statement\n\nHidden.\n-->\n",
            10,
            "unit.md",
        );

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "raw_html_forbidden")
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_body_section")
        );
    }

    #[test]
    fn raw_html_spelling_inside_fenced_code_is_allowed() {
        let diagnostics = validate_metadata(
            &metadata_for_test(),
            "## Statement\n\n```html\n<div>Rendered as code</div>\n```\n",
            1,
            "unit.md",
        );

        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "raw_html_forbidden")
        );
    }

    #[test]
    fn body_diagnostic_lines_are_file_relative() {
        let diagnostics = validate_metadata(&metadata_for_test(), "## Statement\n", 12, "unit.md");
        let empty = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "empty_body_section")
            .expect("empty Statement diagnostic");

        assert_eq!(empty.line, Some(12));
    }

    #[test]
    fn body_start_line_counts_trailing_blank_frontmatter_lines() {
        let content = "---\nschema: example\n\n---\n## Statement\n";
        let (_, body) = super::split_frontmatter(content).expect("frontmatter");

        assert_eq!(super::body_start_line(content, body), 5);
    }

    fn metadata_for_test() -> crate::model::KnowledgeMetadata {
        crate::model::KnowledgeMetadata {
            schema: "methexis.knowledge/v1alpha1".to_owned(),
            id: "tui.example".to_owned(),
            kind: crate::model::KnowledgeKind::Rule,
            owner: "tui-architecture".to_owned(),
            sources: vec![source_ref("tui.fixture")],
            relations: crate::model::Relations::default(),
        }
    }

    fn source_ref(id: &str) -> crate::model::SourceRef {
        crate::model::SourceRef {
            id: id.to_owned(),
            revision: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
        }
    }

    fn parse_for_test(content: &str) -> String {
        let normalized = content.replace("\r\n", "\n");
        let (frontmatter, body) = super::split_frontmatter(&normalized).expect("frontmatter");
        let metadata =
            super::parse_yaml(frontmatter, "test.md", 1).expect("valid test frontmatter");
        knowledge_revision(&metadata, body)
    }
}
