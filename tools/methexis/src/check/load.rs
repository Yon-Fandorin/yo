//! Authority-root inspection, file collection, and foundation loading.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    Diagnostic, Foundation,
    diagnostic::{display_path, local_diagnostic, sort_diagnostics},
    record::{parse_knowledge_file, parse_owner_file},
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
    let sources = match crate::source::load(repository_root) {
        Ok(sources) => sources,
        Err(mut source_diagnostics) => {
            diagnostics.append(&mut source_diagnostics);
            Vec::new()
        },
    };
    let negative_records = match crate::source::negative::load(repository_root) {
        Ok(records) => records,
        Err(mut record_diagnostics) => {
            diagnostics.append(&mut record_diagnostics);
            crate::source::NegativeRecords::empty()
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
    if diagnostics.is_empty() {
        Ok(Foundation {
            units,
            owners,
            sources,
            negative_records,
        })
    } else {
        Err(diagnostics)
    }
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
