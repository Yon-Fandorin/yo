use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::de;

use super::{
    Diagnostic, Foundation, MAX_RECORD_BYTES, body_start_line, display_path, is_segment,
    knowledge_revision, local_diagnostic, sort_diagnostics, validate_metadata,
};
use crate::{
    model::{KnowledgeMetadata, KnowledgeUnit, OWNER_SCHEMA, Owner, OwnerRecord},
    source,
};

pub(super) fn load_records(repository_root: &Path) -> Result<Foundation, Vec<Diagnostic>> {
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
    let sources = match source::load(repository_root) {
        Ok(sources) => sources,
        Err(mut source_diagnostics) => {
            diagnostics.append(&mut source_diagnostics);
            Vec::new()
        },
    };
    let negative_records = match source::negative::load(repository_root) {
        Ok(records) => records,
        Err(mut record_diagnostics) => {
            diagnostics.append(&mut record_diagnostics);
            source::NegativeRecords::empty()
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

    Ok(Foundation {
        units,
        owners,
        sources,
        negative_records,
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
    entries.sort_by_key(fs::DirEntry::path);

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

pub(super) fn normalize_line_endings(content: String) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
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
    T: de::DeserializeOwned,
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
