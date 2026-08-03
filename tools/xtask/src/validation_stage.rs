use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize, de::IgnoredAny};

pub(crate) fn run_methexis_check(repository: &Path) -> Result<(), String> {
    let authority = run_staged_activation_check(repository)?;
    report_prospective_activation(authority);
    Ok(())
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

fn run_staged_activation_check(repository: &Path) -> Result<Authority, String> {
    let output = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "--locked",
            "-p",
            "methexis",
            "--",
            "check",
            "--staged-activation",
        ])
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
