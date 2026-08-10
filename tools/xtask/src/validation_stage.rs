use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize, de::IgnoredAny};

pub(crate) fn run_methexis_check(repository: &Path) -> Result<(), String> {
    let mode = validation_mode(repository)?;
    if mode == ValidationMode::SemanticCandidate {
        require_exact_semantic_worktree(repository)?;
    }
    let authority = run_selected_check(repository, mode)?;
    report_prospective_activation(authority);
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValidationMode {
    SemanticCandidate,
    Complete,
}

fn validation_mode(repository: &Path) -> Result<ValidationMode, String> {
    validation_mode_with_environment(repository, GitEnvironment::Caller)
}

#[derive(Clone, Copy)]
enum GitEnvironment {
    Caller,
    #[cfg(test)]
    Isolated,
}

impl GitEnvironment {
    fn is_isolated(self) -> bool {
        match self {
            Self::Caller => false,
            #[cfg(test)]
            Self::Isolated => true,
        }
    }
}

fn validation_mode_with_environment(
    repository: &Path,
    environment: GitEnvironment,
) -> Result<ValidationMode, String> {
    let output = git_command(repository, environment)
        .args([
            "diff",
            "--cached",
            "--no-renames",
            "--name-only",
            "-z",
            "--diff-filter=ACDMRTUXB",
            "--",
            "methexis",
        ])
        .output()
        .map_err(|error| format!("cannot inspect staged Methexis paths: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot inspect staged Methexis paths: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(classify_staged_paths(&output.stdout))
}

fn classify_staged_paths(paths: &[u8]) -> ValidationMode {
    let mut paths = paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .peekable();
    if paths.peek().is_none() {
        return ValidationMode::Complete;
    }
    if paths.all(|path| {
        path.starts_with(b"methexis/sources/") || path.starts_with(b"methexis/knowledge/")
    }) {
        ValidationMode::SemanticCandidate
    } else {
        ValidationMode::Complete
    }
}

fn require_exact_semantic_worktree(repository: &Path) -> Result<(), String> {
    require_exact_semantic_worktree_with_environment(repository, GitEnvironment::Caller)
}

fn require_exact_semantic_worktree_with_environment(
    repository: &Path,
    environment: GitEnvironment,
) -> Result<(), String> {
    let diff = git_command(repository, environment)
        .args(["diff", "--quiet", "--exit-code", "--", "methexis"])
        .status()
        .map_err(|error| format!("cannot compare staged and working Methexis bytes: {error}"))?;
    match diff.code() {
        Some(0) => {},
        Some(1) => {
            return Err(
                "semantic Methexis candidate has unstaged tracked changes; stage the exact candidate or revert those changes"
                    .to_owned(),
            );
        },
        _ => {
            return Err(format!(
                "cannot compare staged and working Methexis bytes: {diff}"
            ));
        },
    }

    let untracked = untracked_methexis_paths(repository, false, environment)?;
    let ignored = untracked_methexis_paths(repository, true, environment)?;
    if !untracked.is_empty() || !ignored.is_empty() {
        return Err(
            "semantic Methexis candidate has untracked or ignored Methexis paths; stage the exact candidate or remove those paths"
                .to_owned(),
        );
    }
    Ok(())
}

fn untracked_methexis_paths(
    repository: &Path,
    ignored: bool,
    environment: GitEnvironment,
) -> Result<Vec<u8>, String> {
    let mut arguments = vec!["ls-files", "--others", "--exclude-standard", "-z"];
    if ignored {
        arguments.push("--ignored");
    }
    arguments.extend(["--", "methexis"]);
    let output = git_command(repository, environment)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot inspect untracked Methexis paths: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot inspect untracked Methexis paths: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn git_command(repository: &Path, environment: GitEnvironment) -> Command {
    let mut command = Command::new("git");
    command.current_dir(repository).stdin(Stdio::null());
    if environment.is_isolated() {
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_PREFIX",
        ] {
            command.env_remove(variable);
        }
    }
    command
}

fn report_prospective_activation(authority: Authority) {
    if authority == Authority::Prospective {
        println!(
            "prospective Methexis activation validated; ordinary Methexis tests are \
             deferred for this exact staged interval and must run after integration"
        );
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Authority {
    Draft,
    Prospective,
}

#[derive(Deserialize)]
#[serde(tag = "schema")]
enum StageReport {
    #[serde(rename = "methexis.check/v1alpha1")]
    Ordinary {
        ok: bool,
        authority: Authority,
        checks: Vec<IgnoredAny>,
        units: Vec<IgnoredAny>,
        diagnostics: Vec<IgnoredAny>,
    },
    #[serde(rename = "methexis.prospective-activation/v1alpha1")]
    Prospective {
        ok: bool,
        authority: Authority,
        affected_ids: Vec<IgnoredAny>,
    },
}

#[derive(Serialize)]
struct CheckSummary {
    schema: &'static str,
    ok: bool,
    authority: Authority,
    checks: usize,
    units: usize,
    diagnostics: usize,
}

impl StageReport {
    fn summary(&self) -> Result<CheckSummary, String> {
        let (ok, authority, checks, units, diagnostics) = match self {
            Self::Ordinary {
                ok,
                authority,
                checks,
                units,
                diagnostics,
            } => (
                *ok,
                *authority,
                checks.len(),
                units.len(),
                diagnostics.len(),
            ),
            Self::Prospective {
                ok,
                authority,
                affected_ids,
            } => (*ok, *authority, 1, affected_ids.len(), 0),
        };
        let expected_authority = match self {
            Self::Ordinary { .. } => Authority::Draft,
            Self::Prospective { .. } => Authority::Prospective,
        };
        if authority != expected_authority {
            return Err("staged Methexis report schema and authority disagree".to_owned());
        }
        Ok(CheckSummary {
            schema: "yo.methexis-stage-summary/v1",
            ok,
            authority,
            checks,
            units,
            diagnostics,
        })
    }
}

fn run_selected_check(repository: &Path, mode: ValidationMode) -> Result<Authority, String> {
    let output = Command::new("cargo")
        .args(methexis_check_arguments(mode))
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run staged Methexis validation: {error}"))?;

    handle_staged_check_output(
        output.status.success(),
        &output.status.to_string(),
        &output.stdout,
        &output.stderr,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

fn methexis_check_arguments(mode: ValidationMode) -> Vec<&'static str> {
    let mut arguments = vec![
        "run", "--quiet", "--locked", "-p", "methexis", "--", "check",
    ];
    match mode {
        ValidationMode::SemanticCandidate => {
            arguments.extend(["--only", "records,relations"]);
        },
        ValidationMode::Complete => arguments.push("--staged-activation"),
    }
    arguments
}

fn handle_staged_check_output(
    succeeded: bool,
    status: &str,
    captured_stdout: &[u8],
    captured_stderr: &[u8],
    forwarded_stdout: &mut impl Write,
    forwarded_stderr: &mut impl Write,
) -> Result<Authority, String> {
    forwarded_stderr
        .write_all(captured_stderr)
        .map_err(|error| format!("cannot forward Methexis validation diagnostics: {error}"))?;
    if !succeeded {
        forwarded_stdout
            .write_all(captured_stdout)
            .map_err(|error| format!("cannot forward Methexis validation output: {error}"))?;
        return Err(format!("staged Methexis validation failed with {status}"));
    }

    let report = serde_json::from_slice::<StageReport>(captured_stdout).map_err(|error| {
        format!("staged Methexis validation returned an invalid report: {error}")
    })?;
    let summary = report.summary()?;
    if !summary.ok {
        return Err("successful Methexis process returned `ok: false`".to_owned());
    }
    let authority = summary.authority;
    let summary = serde_json::to_string(&summary)
        .map_err(|error| format!("cannot encode Methexis validation summary: {error}"))?;
    writeln!(forwarded_stdout, "{summary}")
        .map_err(|error| format!("cannot forward Methexis validation summary: {error}"))?;
    Ok(authority)
}

#[cfg(test)]
mod tests;
