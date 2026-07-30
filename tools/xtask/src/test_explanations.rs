use std::{
    ffi::OsString,
    io::ErrorKind,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

pub(crate) fn check(repository: &Path) -> Result<(), String> {
    check_from(repository, true)
}

fn check_from(directory: &Path, inherit_git_environment: bool) -> Result<(), String> {
    let root = repository_root(directory, inherit_git_environment)?;
    check_in(&root, inherit_git_environment)
}

fn repository_root(directory: &Path, inherit_git_environment: bool) -> Result<PathBuf, String> {
    let mut output = crate::git::output_bytes_in(
        directory,
        &["rev-parse", "--show-toplevel"],
        inherit_git_environment,
    )?;
    while output
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        output.pop();
    }
    if output.is_empty() {
        return Err("git rev-parse --show-toplevel returned an empty path".to_owned());
    }
    Ok(PathBuf::from(OsString::from_vec(output)))
}

fn check_in(repository: &Path, inherit_git_environment: bool) -> Result<(), String> {
    let paths = rust_source_paths(repository, inherit_git_environment)?;
    check_paths(repository, &paths)
}

fn rust_source_paths(
    repository: &Path,
    inherit_git_environment: bool,
) -> Result<Vec<PathBuf>, String> {
    let output = crate::git::output_bytes_in(
        repository,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "crates",
            "tools",
        ],
        inherit_git_environment,
    )?;
    let mut paths = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn check_paths(repository: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let mut diagnostics = Vec::new();
    for relative in paths {
        let path = repository.join(relative);
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "cannot read Rust source {}: {error}",
                    path.display()
                ));
            },
        };
        let mut previous = "";
        for (index, line) in source.lines().enumerate() {
            if is_test_attribute(line) && !previous.trim_start().starts_with("//") {
                diagnostics.push(format!(
                    "{}:{}: #[test] requires an explanatory line-comment immediately above it; \
                     review verifies Korean readability",
                    relative.display(),
                    index + 1
                ));
            }
            previous = line;
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics.join("\n"))
    }
}

fn is_test_attribute(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("#[test]")
        .is_some_and(|suffix| {
            suffix.is_empty() || suffix.chars().next().is_some_and(char::is_whitespace)
        })
}

#[cfg(test)]
mod tests;
