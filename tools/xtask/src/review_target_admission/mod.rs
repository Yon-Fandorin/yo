mod model;
mod usage;

#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use model::{AccountLimit, Availability, Decision, REQUEST_SCHEMA, RESULT_SCHEMA, Request};
pub(crate) use model::{Admission, ReviewTarget};
use yo_core::{AccountId, LocalConnectionRepository, ModelId, ModelRequestFailureKind, ProviderId};

use crate::bounded_file;

const REQUEST_LIMIT: usize = 64 * 1024;
const MAX_TOKEN_BYTES: usize = 128;
const HOST_VERSION_LIMIT: usize = 256;
const HOST_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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
        ReviewTarget::DelegatedHost { host } => host_availability(host),
    };
    let decision = if availability.state == "unavailable" {
        Decision::Stop
    } else {
        Decision::Admit
    };
    let (status, next_action) = if matches!(decision, Decision::Admit) {
        request.target.admitted_outcome()
    } else {
        ("stopped", "select_human_authorized_alternative")
    };
    let (usage_search, last_exact_usage_receipt) =
        usage::latest_receipt(request.session_repository_path.as_deref(), &request.target);
    Ok(Admission {
        schema: RESULT_SCHEMA,
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
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported external review target admission request schema `{}`; expected `{REQUEST_SCHEMA}`",
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

fn host_availability(host: &str) -> Availability {
    match probe_host_version(host) {
        Ok((executable, version)) => Availability {
            state: "available",
            source: "delegated_host_executable_version",
            failure_kind: None,
            observed_at: None,
            version: Some(version),
            executable: Some(executable),
            detail: "the executable answered its bounded version probe; account usage and entitlement remain host-owned"
                .to_owned(),
        },
        Err(error) => Availability {
            state: "unavailable",
            source: "delegated_host_executable_version",
            failure_kind: Some("local_configuration".to_owned()),
            observed_at: None,
            version: None,
            executable: None,
            detail: error,
        },
    }
}

fn probe_host_version(host: &str) -> Result<(String, String), String> {
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
    Ok((
        executable.to_string_lossy().into_owned(),
        version.to_owned(),
    ))
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
