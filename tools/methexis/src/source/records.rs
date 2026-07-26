//! Typed Source record loading.

use std::{collections::BTreeMap, path::Path};

use crate::{
    check::{
        Diagnostic, DiagnosticPhase, collect_files, normalize_record_bytes, parse_yaml,
        read_normalized,
    },
    model::{Source, SourceRecord},
};

pub(crate) fn load(repository_root: &Path) -> Result<Vec<Source>, Vec<Diagnostic>> {
    load_impl(repository_root, false).map(|(sources, _)| sources)
}

pub(crate) fn load_captured(
    repository_root: &Path,
) -> Result<(Vec<Source>, Vec<super::working_tree::Capture>), Vec<Diagnostic>> {
    load_impl(repository_root, true)
}

fn load_impl(
    repository_root: &Path,
    capture: bool,
) -> Result<(Vec<Source>, Vec<super::working_tree::Capture>), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut paths_by_id = BTreeMap::<String, String>::new();
    let mut captures = Vec::new();
    let paths = collect_files(
        &repository_root.join("methexis/sources"),
        "yaml",
        repository_root,
        &mut diagnostics,
    );
    let mut sources = Vec::new();
    for path in paths {
        let display = relative_path(repository_root, &path);
        let content = if capture {
            match super::working_tree::capture_record(repository_root, &display) {
                Ok((bytes, record_capture)) => {
                    captures.push(record_capture);
                    normalize_record_bytes(&bytes, &display)
                },
                Err(failure) => Err(vec![capture_diagnostic(&display, failure)]),
            }
        } else {
            read_normalized(&path, &display)
        };
        match content.and_then(|content| parse_yaml::<SourceRecord>(&content, &display, 0)) {
            Ok(record) => {
                let record_diagnostics = super::validation::validate(&record, &display);
                if record_diagnostics.is_empty() {
                    if let Some(previous) = paths_by_id.insert(record.id.clone(), display.clone()) {
                        diagnostics.push(duplicate(&previous, &record.id));
                        diagnostics.push(duplicate(&display, &record.id));
                    }
                    sources.push(Source { record, path });
                } else {
                    diagnostics.extend(record_diagnostics);
                }
            },
            Err(mut errors) => diagnostics.append(&mut errors),
        }
    }
    diagnostics.sort_by(|left, right| {
        (
            left.phase,
            &left.path,
            &left.code,
            left.line,
            left.column,
            &left.message,
        )
            .cmp(&(
                right.phase,
                &right.path,
                &right.code,
                right.line,
                right.column,
                &right.message,
            ))
    });
    diagnostics.dedup();
    if diagnostics.is_empty() {
        Ok((sources, captures))
    } else {
        Err(diagnostics)
    }
}

fn capture_diagnostic(path: &str, failure: super::FreshnessFailure) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Local,
        path: path.to_owned(),
        code: failure.code.to_owned(),
        message: failure.message,
        line: None,
        column: None,
        affected_ids: failure.affected_ids,
    }
}

fn duplicate(path: &str, id: &str) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path: path.to_owned(),
        code: "duplicate_source_id".to_owned(),
        message: format!("SourceId `{id}` is defined more than once"),
        line: None,
        column: None,
        affected_ids: vec![id.to_owned()],
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
