mod model;
mod usage;

#[cfg(test)]
mod tests;

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use model::{
    AccountLimit, Availability, Decision, REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2,
    REQUEST_SCHEMA_V1_ALPHA3, REQUEST_SCHEMA_V1_ALPHA4, Request, result_schema,
};
pub(crate) use model::{Admission, ReviewTarget};
use yo_core::{AccountId, LocalConnectionRepository, ModelId, ModelRequestFailureKind, ProviderId};

use crate::bounded_file;

const REQUEST_LIMIT: usize = 64 * 1024;
const MAX_TOKEN_BYTES: usize = 128;
const HOST_VERSION_LIMIT: usize = 256;
const HOST_DIAGNOSTIC_LIMIT: usize = 8 * 1024;
const HOST_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
static HOST_STATE_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn run(request_path: &Path) -> Result<(), String> {
    let admission = evaluate(request_path)?;
    println!(
        "{}",
        serde_json::to_string(&admission)
            .map_err(|error| format!("cannot encode review-target admission result: {error}"))?
    );
    Ok(())
}

pub(crate) fn evaluate(request_path: &Path) -> Result<Admission, String> {
    let bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "external review target admission request",
    )?;
    let request: Request = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid external review target admission request {}: {error}",
            request_path.display()
        )
    })?;
    validate_request(&request)?;
    evaluate_request(&request)
}

fn evaluate_request(request: &Request) -> Result<Admission, String> {
    let availability = match &request.target {
        ReviewTarget::ManagedModel {
            provider,
            account,
            model,
        } => managed_availability(
            request
                .connection_repository_path
                .as_deref()
                .expect("validated managed request has a connection repository"),
            provider,
            account,
            model,
        ),
        ReviewTarget::DelegatedHost { host } => host_availability(
            host,
            match request.schema.as_str() {
                REQUEST_SCHEMA_V1_ALPHA4 => HostReadiness::ExecutionProfile,
                REQUEST_SCHEMA_V1_ALPHA3 => HostReadiness::State,
                _ => HostReadiness::Version,
            },
        ),
    };
    let decision = if availability.state == "unavailable" {
        Decision::Stop
    } else {
        Decision::Admit
    };
    let (status, next_action) = if matches!(decision, Decision::Admit) {
        request.target.admitted_outcome(&request.schema)
    } else {
        ("stopped", "select_human_authorized_alternative")
    };
    let (usage_search, last_exact_usage_receipt) =
        usage::latest_receipt(request.session_repository_path.as_deref(), &request.target);
    Ok(Admission {
        schema: result_schema(&request.schema),
        ok: matches!(decision, Decision::Admit),
        status,
        next_action,
        decision,
        target_reference: request.target.reference(),
        target: request.target.clone(),
        availability,
        account_limit: AccountLimit {
            availability: "unknown",
            remaining: None,
            resets_at: None,
            source: None,
        },
        usage_search,
        last_exact_usage_receipt,
    })
}

fn validate_request(request: &Request) -> Result<(), String> {
    if !matches!(
        request.schema.as_str(),
        REQUEST_SCHEMA
            | REQUEST_SCHEMA_V1_ALPHA2
            | REQUEST_SCHEMA_V1_ALPHA3
            | REQUEST_SCHEMA_V1_ALPHA4
    ) {
        return Err(format!(
            "unsupported external review target admission request schema `{}`; expected `{REQUEST_SCHEMA}`, `{REQUEST_SCHEMA_V1_ALPHA2}`, `{REQUEST_SCHEMA_V1_ALPHA3}`, or `{REQUEST_SCHEMA_V1_ALPHA4}`",
            request.schema
        ));
    }
    validate_target(&request.target)?;
    match (&request.target, &request.connection_repository_path) {
        (ReviewTarget::ManagedModel { .. }, Some(path)) => {
            compact_absolute_path(path, "connection_repository_path")?
        },
        (ReviewTarget::ManagedModel { .. }, None) => {
            return Err("a managed-model admission requires connection_repository_path".to_owned());
        },
        (ReviewTarget::DelegatedHost { .. }, Some(_)) => {
            return Err(
                "a delegated-host admission must not name connection_repository_path".to_owned(),
            );
        },
        (ReviewTarget::DelegatedHost { .. }, None) => {},
    }
    if let Some(path) = &request.session_repository_path {
        compact_absolute_path(path, "session_repository_path")?;
    }
    Ok(())
}

pub(crate) fn validate_target(target: &ReviewTarget) -> Result<(), String> {
    match target {
        ReviewTarget::ManagedModel {
            provider,
            account,
            model,
        } => {
            compact_token(provider, "target provider")?;
            compact_token(account, "target account")?;
            compact_token(model, "target model")?;
            if [provider, account, model]
                .into_iter()
                .any(|value| value.contains(':'))
            {
                return Err("managed review-target coordinates must not contain `:`".to_owned());
            }
            ProviderId::new(provider.clone()).map_err(|error| error.to_string())?;
            AccountId::new(account.clone()).map_err(|error| error.to_string())?;
            ModelId::new(model.clone()).map_err(|error| error.to_string())?;
        },
        ReviewTarget::DelegatedHost { host } => {
            compact_token(host, "target host")?;
            if !matches!(host.as_str(), "codex" | "grok") {
                return Err(
                    "delegated review target must be exact host `codex` or `grok`".to_owned(),
                );
            }
        },
    }
    Ok(())
}

fn managed_availability(path: &str, provider: &str, account: &str, model: &str) -> Availability {
    let repository = LocalConnectionRepository::new(PathBuf::from(path));
    let snapshot = match repository.capture() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Availability {
                state: "unavailable",
                source: "connection_repository",
                failure_kind: Some("local_configuration".to_owned()),
                observed_at: None,
                version: None,
                executable: None,
                detail: format!("cannot read the exact connection repository: {error}"),
            };
        },
    };
    let selected = snapshot.models().iter().find(|stored| {
        let binding = stored.complete().binding();
        binding.provider_id().as_str() == provider
            && binding.account_id().as_str() == account
            && binding.model_id().as_str() == model
    });
    let Some(selected) = selected else {
        return Availability {
            state: "unavailable",
            source: "connection_repository",
            failure_kind: Some("local_configuration".to_owned()),
            observed_at: None,
            version: None,
            executable: None,
            detail: "the exact managed review target is not stored".to_owned(),
        };
    };
    let Some(failure) = selected.last_failure() else {
        return Availability {
            state: "unknown",
            source: "connection_repository",
            failure_kind: None,
            observed_at: None,
            version: None,
            executable: None,
            detail:
                "the binding is stored, but no request-free entitlement or quota proof is available"
                    .to_owned(),
        };
    };
    let blocking = blocking_failure(failure.kind());
    Availability {
        state: if blocking { "unavailable" } else { "unknown" },
        source: "connection_repository.last_failure",
        failure_kind: Some(failure.kind().as_str().to_owned()),
        observed_at: Some(failure.observed_at().to_owned()),
        version: None,
        executable: None,
        detail: if blocking {
            "the newest typed target observation is unavailable; a successful exact request must clear it"
                .to_owned()
        } else {
            "the newest typed failure is reported without inferring quota exhaustion or current unavailability"
                .to_owned()
        },
    }
}

const fn blocking_failure(kind: ModelRequestFailureKind) -> bool {
    matches!(
        kind,
        ModelRequestFailureKind::Authentication
            | ModelRequestFailureKind::AccessDenied
            | ModelRequestFailureKind::ModelUnavailable
            | ModelRequestFailureKind::LocalConfiguration
    )
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HostReadiness {
    Version,
    State,
    ExecutionProfile,
}

fn host_availability(host: &str, readiness: HostReadiness) -> Availability {
    match probe_host(host, readiness) {
        Ok((executable, version)) => Availability {
            state: "available",
            source: match readiness {
                HostReadiness::Version => "delegated_host_executable_version",
                HostReadiness::State => "delegated_host_executable_and_state_readiness",
                HostReadiness::ExecutionProfile => {
                    "delegated_host_executable_state_and_execution_profile_readiness"
                },
            },
            failure_kind: None,
            observed_at: None,
            version: Some(version),
            executable: Some(executable),
            detail: match readiness {
                HostReadiness::Version => "the executable answered its bounded version probe; account usage and entitlement remain host-owned".to_owned(),
                HostReadiness::State => "the executable answered its bounded version probe and its existing host-state directory passed a create-and-remove probe; account usage and entitlement remain host-owned".to_owned(),
                HostReadiness::ExecutionProfile => "the executable answered its bounded version probe, its existing host-state directory passed a create-and-remove probe, and the exact request-free execution profile started successfully; account usage and entitlement remain host-owned".to_owned(),
            },
        },
        Err(error) => Availability {
            state: "unavailable",
            source: match readiness {
                HostReadiness::Version => "delegated_host_executable_version",
                HostReadiness::State => "delegated_host_executable_and_state_readiness",
                HostReadiness::ExecutionProfile => {
                    "delegated_host_executable_state_and_execution_profile_readiness"
                },
            },
            failure_kind: Some("local_configuration".to_owned()),
            observed_at: None,
            version: None,
            executable: None,
            detail: error,
        },
    }
}

fn probe_host(host: &str, readiness: HostReadiness) -> Result<(String, String), String> {
    let (executable, version) = probe_host_version(host)?;
    if readiness != HostReadiness::Version {
        let state = host_state_directory(host)?;
        probe_host_state_writable(&state)?;
    }
    if readiness == HostReadiness::ExecutionProfile {
        if host != "grok" {
            return Err(format!(
                "no request-free execution-profile readiness probe is registered for delegated host `{host}`"
            ));
        }
        probe_grok_read_only_startup(&executable)?;
    }
    Ok((executable.to_string_lossy().into_owned(), version))
}

fn probe_grok_read_only_startup(executable: &Path) -> Result<(), String> {
    let mut child = Command::new(executable)
        .args([
            "--sandbox",
            "read-only",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "Read,Grep",
            "--no-subagents",
            "--disable-web-search",
            "agent",
            "stdio",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start Grok's request-free read-only profile: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .expect("piped Grok startup stdout is available");
    let stderr = child
        .stderr
        .take()
        .expect("piped Grok startup stderr is available");
    let stdout_reader = thread::spawn(move || read_bounded_diagnostic(stdout));
    let stderr_reader = thread::spawn(move || read_bounded_diagnostic(stderr));
    let status = wait_for_host_probe(&mut child, "Grok's request-free read-only profile");
    let (stdout, stdout_exceeded) = stdout_reader
        .join()
        .map_err(|_| "cannot collect Grok's request-free read-only profile stdout".to_owned())?;
    let (stderr, stderr_exceeded) = stderr_reader
        .join()
        .map_err(|_| "cannot collect Grok's request-free read-only profile stderr".to_owned())?;
    let status = status?;
    if stdout_exceeded || stderr_exceeded {
        return Err(format!(
            "Grok's request-free read-only profile exceeded the {HOST_DIAGNOSTIC_LIMIT}-byte per-stream diagnostic limit"
        ));
    }
    if status.success() {
        return Ok(());
    }
    let diagnostic = [stdout, stderr]
        .into_iter()
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| compact_diagnostic(&bytes))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if diagnostic.is_empty() {
        Err(format!(
            "Grok's request-free read-only profile exited without success ({status})"
        ))
    } else {
        Err(format!(
            "Grok's request-free read-only profile exited without success ({status}): {diagnostic}"
        ))
    }
}

fn compact_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_bounded_diagnostic(mut reader: impl Read) -> (Vec<u8>, bool) {
    let mut retained = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 1024];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let remaining = HOST_DIAGNOSTIC_LIMIT.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
        exceeded |= read > remaining;
    }
    (retained, exceeded)
}

fn wait_for_host_probe(child: &mut Child, label: &str) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + HOST_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} exceeded its 10-second deadline"));
            },
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot observe {label}: {error}"));
            },
        }
    }
}

fn host_state_directory(host: &str) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "HOME is unavailable for delegated-host state readiness".to_owned())?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err("HOME must be absolute for delegated-host state readiness".to_owned());
    }
    Ok(home.join(match host {
        "codex" => ".codex",
        "grok" => ".grok",
        _ => unreachable!("validated delegated host"),
    }))
}

fn probe_host_state_writable(directory: &Path) -> Result<(), String> {
    let metadata = fs::metadata(directory).map_err(|error| {
        format!(
            "cannot inspect delegated-host state directory {}: {error}",
            directory.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "delegated-host state path {} is not a directory",
            directory.display()
        ));
    }
    let sequence = HOST_STATE_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(".yo-readiness-{}-{sequence}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "delegated-host state directory {} is not writable: {error}",
                directory.display()
            )
        })?;
    if let Err(error) = file.write_all(b"yo delegated-host readiness\n") {
        let _ = fs::remove_file(&path);
        return Err(format!(
            "cannot write delegated-host readiness sentinel {}: {error}",
            path.display()
        ));
    }
    drop(file);
    fs::remove_file(&path).map_err(|error| {
        format!(
            "cannot remove delegated-host readiness sentinel {}: {error}",
            path.display()
        )
    })
}

fn probe_host_version(host: &str) -> Result<(PathBuf, String), String> {
    let executable = resolve_executable(host)?;
    let mut child = Command::new(&executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start `{host} --version`: {error}"))?;
    let deadline = Instant::now() + HOST_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                return Err(format!(
                    "`{host} --version` exited without success ({status})"
                ));
            },
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`{host} --version` exceeded its 10-second deadline"
                ));
            },
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot observe `{host} --version`: {error}"));
            },
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot collect `{host} --version`: {error}"))?;
    if output.stdout.len() > HOST_VERSION_LIMIT {
        return Err(format!(
            "`{host} --version` exceeded the {HOST_VERSION_LIMIT}-byte output limit"
        ));
    }
    let version = std::str::from_utf8(&output.stdout)
        .map_err(|_| format!("`{host} --version` did not return UTF-8"))?
        .trim();
    if version.is_empty()
        || version.contains(['\r', '\n'])
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return Err(format!(
            "`{host} --version` must return one non-empty visible ASCII line"
        ));
    }
    Ok((executable, version.to_owned()))
}

fn resolve_executable(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| "PATH is unavailable for delegated-host admission".to_owned())?;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(name);
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if metadata.is_file() {
            return fs::canonicalize(&candidate).map_err(|error| {
                format!(
                    "cannot resolve delegated-host executable {}: {error}",
                    candidate.display()
                )
            });
        }
    }
    Err(format!(
        "delegated-host executable `{name}` was not found on absolute PATH entries"
    ))
}

fn compact_path(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!(
            "{label} must be a non-empty path of at most 4096 bytes"
        ))
    } else {
        Ok(())
    }
}

fn compact_absolute_path(value: &str, label: &str) -> Result<(), String> {
    compact_path(value, label)?;
    if Path::new(value).is_absolute() {
        Ok(())
    } else {
        Err(format!("{label} must be absolute"))
    }
}

fn compact_token(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_TOKEN_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_whitespace())
    {
        Err(format!(
            "{label} must be a non-empty visible ASCII token of at most {MAX_TOKEN_BYTES} bytes"
        ))
    } else {
        Ok(())
    }
}
