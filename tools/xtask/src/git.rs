use std::{
    ffi::OsStr,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub(crate) fn output_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<String, String> {
    let output = output_bytes_in(directory, arguments, inherit_repository_environment)?;
    String::from_utf8(output).map_err(|error| {
        format!(
            "git {} returned non-UTF-8 output: {error}",
            arguments.join(" ")
        )
    })
}

pub(crate) fn output_bytes_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<Vec<u8>, String> {
    let result = command(directory, inherit_repository_environment)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "git {} failed with {}{}",
            arguments.join(" "),
            result.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))
    }
}

pub(crate) fn output_bytes_in_with_index(
    directory: &Path,
    arguments: &[&str],
    index_file: Option<&OsStr>,
) -> Result<Vec<u8>, String> {
    let mut command = command(directory, false);
    if let Some(index_file) = index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    let result = command
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "git {} failed with {}{}",
            arguments.join(" "),
            result.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))
    }
}

pub(crate) fn optional_output_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<Option<String>, String> {
    let result = command(directory, inherit_repository_environment)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        String::from_utf8(result.stdout).map(Some).map_err(|error| {
            format!(
                "git {} returned non-UTF-8 output: {error}",
                arguments.join(" ")
            )
        })
    } else {
        Ok(None)
    }
}

pub(crate) fn succeeds_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<bool, String> {
    command(directory, inherit_repository_environment)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))
}

fn command(directory: &Path, inherit_repository_environment: bool) -> Command {
    let mut command = Command::new("git");
    command.current_dir(directory);
    if !inherit_repository_environment {
        command
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE");
    }
    command
}

pub(crate) fn interpret_trailers(message: &str) -> Result<String, String> {
    let mut child = Command::new("git")
        .args(["interpret-trailers", "--parse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start git interpret-trailers: {error}"))?;
    child
        .stdin
        .take()
        .expect("piped stdin is available")
        .write_all(message.as_bytes())
        .map_err(|error| {
            format!("cannot pass the commit message to git interpret-trailers: {error}")
        })?;
    let result = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for git interpret-trailers: {error}"))?;
    if !result.status.success() {
        return Err("git interpret-trailers could not parse the commit message".to_owned());
    }
    String::from_utf8(result.stdout)
        .map_err(|error| format!("git interpret-trailers returned non-UTF-8 output: {error}"))
}

pub(crate) fn read(path: &Path, label: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))
}
