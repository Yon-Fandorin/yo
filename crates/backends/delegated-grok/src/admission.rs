//! Grok configuration and bounded-review admission checks.

use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use yo_core::{BackendFailure, BackendFailureKind};

use crate::config::{
    GrokBackendConfig, OUTER_SANDBOX_REVIEW_ENV, OUTER_SANDBOX_REVIEW_PROFILE,
    OUTER_SANDBOX_SENTINEL,
};

static OUTER_SANDBOX_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn validate_config(config: &GrokBackendConfig) -> Result<(), BackendFailure> {
    if !config.working_directory().is_absolute()
        || !config.working_directory().is_dir()
        || config.request_timeout().is_zero()
    {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            "Grok requires an existing absolute working directory and a non-zero request timeout",
        ));
    }
    if config.outer_sandboxed_review() {
        verify_outer_sandbox(config)?;
    }
    Ok(())
}

fn verify_outer_sandbox(config: &GrokBackendConfig) -> Result<(), BackendFailure> {
    if !config.read_only_review()
        || std::env::var_os(OUTER_SANDBOX_REVIEW_ENV).as_deref()
            != Some(std::ffi::OsStr::new(OUTER_SANDBOX_REVIEW_PROFILE))
    {
        return Err(initialization_failure(
            "Yo outer Grok review isolation was requested without its exact bounded-runner profile",
        ));
    }
    #[cfg(not(target_os = "linux"))]
    return Err(initialization_failure(
        "Yo outer Grok review isolation is supported only on Linux",
    ));
    #[cfg(target_os = "linux")]
    {
        require_read_only_mount(Path::new(OUTER_SANDBOX_SENTINEL))?;
        require_workspace_write_blocked(config.working_directory())?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn require_read_only_mount(path: &Path) -> Result<(), BackendFailure> {
    if !path.is_dir() {
        return Err(initialization_failure(
            "Yo outer Grok review isolation sentinel is missing",
        ));
    }
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").map_err(|error| {
        initialization_failure(format!(
            "cannot inspect Yo outer Grok review isolation mounts: {error}"
        ))
    })?;
    let expected = path.to_string_lossy();
    let read_only = mountinfo.lines().any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.get(4).is_some_and(|mount| *mount == expected)
            && fields
                .get(5)
                .is_some_and(|options| options.split(',').any(|option| option == "ro"))
    });
    if !read_only {
        return Err(initialization_failure(
            "Yo outer Grok review isolation sentinel is not an exact read-only mount",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn require_workspace_write_blocked(workspace: &Path) -> Result<(), BackendFailure> {
    let sequence = OUTER_SANDBOX_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe = workspace.join(format!(
        ".yo-grok-outer-sandbox-write-probe-{}-{sequence}",
        std::process::id()
    ));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let cleanup = fs::remove_file(&probe).err();
            let detail = cleanup.map_or_else(String::new, |error| {
                format!("; the unexpected probe also could not be removed: {error}")
            });
            Err(initialization_failure(format!(
                "Yo outer Grok review isolation left the workspace writable{detail}"
            )))
        },
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(30) =>
        {
            Ok(())
        },
        Err(error) => Err(initialization_failure(format!(
            "cannot verify that Yo outer Grok review isolation blocks workspace writes: {error}"
        ))),
    }
}

fn initialization_failure(message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(BackendFailureKind::Initialization, message.into())
}
