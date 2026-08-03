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
struct CheckReport {
    ok: bool,
    authority: Authority,
    checks: Vec<IgnoredAny>,
    units: Vec<IgnoredAny>,
    diagnostics: Vec<IgnoredAny>,
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

impl CheckReport {
    fn summary(&self) -> CheckSummary {
        CheckSummary {
            schema: "yo.methexis-stage-summary/v1",
            ok: self.ok,
            authority: self.authority,
            checks: self.checks.len(),
            units: self.units.len(),
            diagnostics: self.diagnostics.len(),
        }
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

    let report = serde_json::from_slice::<CheckReport>(captured_stdout).map_err(|error| {
        format!("staged Methexis validation returned an invalid report: {error}")
    })?;
    if !report.ok {
        return Err("successful Methexis process returned `ok: false`".to_owned());
    }
    let summary = serde_json::to_string(&report.summary())
        .map_err(|error| format!("cannot encode Methexis validation summary: {error}"))?;
    writeln!(forwarded_stdout, "{summary}")
        .map_err(|error| format!("cannot forward Methexis validation summary: {error}"))?;
    Ok(report.authority)
}

#[cfg(test)]
mod tests;
