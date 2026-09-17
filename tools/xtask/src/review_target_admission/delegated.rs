use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process,
    process::{Child, Command, ExitStatus, Stdio},
    str,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use super::model::Availability;
use crate::grok_outer_sandbox;

const HOST_VERSION_LIMIT: usize = 256;
const HOST_DIAGNOSTIC_LIMIT: usize = 8 * 1024;
const HOST_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
static HOST_STATE_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum HostReadiness {
    Version,
    State,
    ExecutionProfile,
    ExecutionIsolation,
}

struct HostProbe {
    executable: String,
    version: String,
    execution_isolation: Option<String>,
    isolation_detail: Option<String>,
}

pub(super) fn host_availability(host: &str, readiness: HostReadiness) -> Availability {
    match probe_host(host, readiness) {
        Ok(probe) => Availability {
            state: "available",
            source: match readiness {
                HostReadiness::Version => "delegated_host_executable_version",
                HostReadiness::State => "delegated_host_executable_and_state_readiness",
                HostReadiness::ExecutionProfile => {
                    "delegated_host_executable_state_and_execution_profile_readiness"
                },
                HostReadiness::ExecutionIsolation => {
                    "delegated_host_executable_state_and_execution_isolation_readiness"
                },
            },
            failure_kind: None,
            observed_at: None,
            failure_freshness: None,
            version: Some(probe.version),
            executable: Some(probe.executable),
            execution_isolation: probe.execution_isolation,
            detail: match readiness {
                HostReadiness::Version => "the executable answered its bounded version probe; account usage and entitlement remain host-owned".to_owned(),
                HostReadiness::State => "the executable answered its bounded version probe and its existing host-state directory passed a create-and-remove probe; account usage and entitlement remain host-owned".to_owned(),
                HostReadiness::ExecutionProfile => "the executable answered its bounded version probe, its existing host-state directory passed a create-and-remove probe, and the exact request-free execution profile started successfully; account usage and entitlement remain host-owned".to_owned(),
                HostReadiness::ExecutionIsolation => probe.isolation_detail.expect("execution-isolation readiness records its selected boundary"),
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
                HostReadiness::ExecutionIsolation => {
                    "delegated_host_executable_state_and_execution_isolation_readiness"
                },
            },
            failure_kind: Some("local_configuration".to_owned()),
            observed_at: None,
            failure_freshness: None,
            version: None,
            executable: None,
            execution_isolation: None,
            detail: error,
        },
    }
}

fn probe_host(host: &str, readiness: HostReadiness) -> Result<HostProbe, String> {
    let (executable, version) = probe_host_version(host)?;
    if readiness != HostReadiness::Version {
        let state = host_state_directory(host)?;
        probe_host_state_writable(&state)?;
    }
    let (execution_isolation, isolation_detail) = if matches!(
        readiness,
        HostReadiness::ExecutionProfile | HostReadiness::ExecutionIsolation
    ) {
        if host != "grok" {
            return Err(format!(
                "no request-free execution-profile readiness probe is registered for delegated host `{host}`"
            ));
        }
        match probe_grok_read_only_startup(&executable) {
            Ok(()) => (
                (readiness == HostReadiness::ExecutionIsolation)
                    .then(|| grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE.to_owned()),
                (readiness == HostReadiness::ExecutionIsolation).then(|| {
                    "the native Grok read-only profile passed the request-free startup probe and remains the selected isolation; account usage and entitlement remain host-owned".to_owned()
                }),
            ),
            Err(native_failure) if readiness == HostReadiness::ExecutionIsolation => {
                probe_grok_outer_read_only_startup(&executable)?;
                (
                    Some(grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE.to_owned()),
                    Some(format!(
                        "the native Grok read-only profile was unavailable ({native_failure}); the Yo-owned bwrap read-only no-tools profile passed its request-free startup probe and was selected; account usage and entitlement remain host-owned"
                    )),
                )
            },
            Err(error) => return Err(error),
        }
    } else {
        (None, None)
    };
    Ok(HostProbe {
        executable: executable.to_string_lossy().into_owned(),
        version,
        execution_isolation,
        isolation_detail,
    })
}

pub(super) fn probe_grok_read_only_startup(executable: &Path) -> Result<(), String> {
    let mut command = Command::new(executable);
    command.args([
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
    ]);
    run_grok_startup(command, "Grok's request-free read-only profile")
}

fn probe_grok_outer_read_only_startup(executable: &Path) -> Result<(), String> {
    let working_directory = env::current_dir()
        .and_then(fs::canonicalize)
        .map_err(|error| format!("cannot resolve the outer-sandbox probe directory: {error}"))?;
    let command = grok_outer_sandbox::probe_command(executable, &working_directory)?;
    run_grok_startup(command, "Yo's request-free outer read-only Grok profile")
}

fn run_grok_startup(mut command: Command, label: &str) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start {label}: {error}"))?;
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
    let status = wait_for_host_probe(&mut child, label);
    let (stdout, stdout_exceeded) = stdout_reader
        .join()
        .map_err(|_| format!("cannot collect {label} stdout"))?;
    let (stderr, stderr_exceeded) = stderr_reader
        .join()
        .map_err(|_| format!("cannot collect {label} stderr"))?;
    let status = status?;
    if stdout_exceeded || stderr_exceeded {
        return Err(format!(
            "{label} exceeded the {HOST_DIAGNOSTIC_LIMIT}-byte per-stream diagnostic limit"
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
        Err(format!("{label} exited without success ({status})"))
    } else {
        Err(format!(
            "{label} exited without success ({status}): {diagnostic}"
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
    let home = env::var_os("HOME")
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

pub(super) fn probe_host_state_writable(directory: &Path) -> Result<(), String> {
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
    let path = directory.join(format!(".yo-readiness-{}-{sequence}", process::id()));
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
    let version = str::from_utf8(&output.stdout)
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
    let path = env::var_os("PATH")
        .ok_or_else(|| "PATH is unavailable for delegated-host admission".to_owned())?;
    for directory in env::split_paths(&path) {
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
