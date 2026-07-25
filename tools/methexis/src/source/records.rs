//! Typed Source record loading.

use std::path::Path;

use crate::{
    check::{Diagnostic, collect_files, parse_yaml, read_normalized},
    model::{Source, SourceRecord},
};

pub(crate) fn load(repository_root: &Path) -> Result<Vec<Source>, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let paths = collect_files(
        &repository_root.join("methexis/sources"),
        "yaml",
        repository_root,
        &mut diagnostics,
    );
    let mut sources = Vec::new();
    for path in paths {
        let display = relative_path(repository_root, &path);
        match read_normalized(&path, &display)
            .and_then(|content| parse_yaml::<SourceRecord>(&content, &display, 0))
        {
            Ok(record) => {
                let record_diagnostics = super::validation::validate(&record, &display);
                if record_diagnostics.is_empty() {
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
    if diagnostics.is_empty() {
        Ok(sources)
    } else {
        Err(diagnostics)
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
