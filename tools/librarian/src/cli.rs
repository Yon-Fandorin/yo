//! Thin process boundary for the agent-first discovery contract.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitCode,
};

use crate::{
    catalog, discovery,
    error::DiscoveryError,
    wire::{COMPILER, DiscoveryRequest},
};

const HELP: &str = "\
Deterministic working-tree Knowledge discovery

Usage:
  librarian discover [--repository <path>] <request.json>
  librarian help
  librarian version

Success is one JSON value on stdout. Failure is one JSON value on stderr.
";
const MAX_REQUEST_BYTES: usize = 256 * 1024;

pub fn run<I, S>(
    arguments: I,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> io::Result<ExitCode>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let arguments = arguments
        .into_iter()
        .map(Into::into)
        .collect::<Vec<OsString>>();
    match arguments.first().and_then(|value| value.to_str()) {
        Some("help" | "--help" | "-h") | None => {
            stdout.write_all(HELP.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        },
        Some("version" | "--version" | "-V") => {
            writeln!(stdout, "{COMPILER}")?;
            Ok(ExitCode::SUCCESS)
        },
        Some("discover") => run_discover(&arguments[1..], stdout, stderr),
        Some(command) => write_failure(
            DiscoveryError::request(
                "unknown_command",
                format!("unknown command `{command}`; run `librarian help`"),
            ),
            &mut stderr,
        ),
    }
}

fn run_discover(
    arguments: &[OsString],
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> io::Result<ExitCode> {
    let (repository_root, request_path) = match parse_discover_arguments(arguments) {
        Ok(value) => value,
        Err(error) => return write_failure(error, &mut stderr),
    };
    let request_bytes = match read_request(&request_path) {
        Ok(bytes) => bytes,
        Err(error) => return write_failure(error, &mut stderr),
    };
    let request: DiscoveryRequest = match serde_json::from_slice(&request_bytes) {
        Ok(request) => request,
        Err(error) => {
            return write_failure(
                DiscoveryError::request(
                    "malformed_request",
                    format!("request is not valid contract JSON: {error}"),
                ),
                &mut stderr,
            );
        },
    };
    if let Err(error) = discovery::validate_request(&request) {
        return write_failure(error, &mut stderr);
    }
    let catalog = match catalog::load(&repository_root) {
        Ok(catalog) => catalog,
        Err(error) => return write_failure(error, &mut stderr),
    };
    let result = match discovery::discover(request, &catalog) {
        Ok(result) => result,
        Err(error) => return write_failure(error, &mut stderr),
    };
    let mut bytes = serde_json::to_vec_pretty(&result)
        .map_err(|error| io::Error::other(format!("cannot serialize candidate set: {error}")))?;
    bytes.push(b'\n');
    stdout.write_all(&bytes)?;
    Ok(ExitCode::SUCCESS)
}

fn read_request(path: &std::path::Path) -> Result<Vec<u8>, DiscoveryError> {
    let mut file = File::open(path).map_err(|error| {
        DiscoveryError::io(
            "request_read_failed",
            format!("cannot read {}: {error}", path.display()),
        )
    })?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            DiscoveryError::io(
                "request_read_failed",
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(DiscoveryError::request(
            "request_too_large",
            "discovery requests must not exceed 256 KiB",
        ));
    }
    Ok(bytes)
}

fn parse_discover_arguments(arguments: &[OsString]) -> Result<(PathBuf, PathBuf), DiscoveryError> {
    let mut repository_root = None;
    let mut request_path = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == "--repository" {
            index += 1;
            let Some(path) = arguments.get(index) else {
                return Err(usage_error("`--repository` requires a path"));
            };
            if repository_root.replace(PathBuf::from(path)).is_some() {
                return Err(usage_error("`--repository` may appear only once"));
            }
        } else if arguments[index]
            .to_str()
            .is_some_and(|value| value.starts_with('-'))
        {
            return Err(usage_error("unknown discover option"));
        } else if request_path
            .replace(PathBuf::from(&arguments[index]))
            .is_some()
        {
            return Err(usage_error("discover accepts exactly one request file"));
        }
        index += 1;
    }
    let request_path =
        request_path.ok_or_else(|| usage_error("discover requires a request file"))?;
    let repository_root = match repository_root {
        Some(path) => path,
        None => std::env::current_dir().map_err(|error| {
            DiscoveryError::io(
                "repository_root_unavailable",
                format!("cannot resolve the current directory: {error}"),
            )
        })?,
    };
    Ok((repository_root, request_path))
}

fn usage_error(message: &'static str) -> DiscoveryError {
    DiscoveryError::request(
        "invalid_arguments",
        format!("{message}; run `librarian help`"),
    )
}

fn write_failure(error: DiscoveryError, mut stderr: impl Write) -> io::Result<ExitCode> {
    let mut bytes = serde_json::to_vec_pretty(&error.into_envelope())
        .map_err(|error| io::Error::other(format!("cannot serialize failure: {error}")))?;
    bytes.push(b'\n');
    stderr.write_all(&bytes)?;
    Ok(ExitCode::from(2))
}

#[cfg(test)]
mod tests;
