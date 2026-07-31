use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use serde::Deserialize;

pub(crate) fn run_methexis_tests(repository: &Path) -> Result<(), String> {
    let authority = run_staged_activation_check(repository)?;
    if authority == Authority::Prospective {
        println!(
            "prospective Methexis activation validated; ordinary Methexis tests are \
             deferred for this exact staged interval and must run after integration"
        );
        return Ok(());
    }

    run_cargo(
        repository,
        &["test", "--quiet", "--locked", "-p", "methexis"],
        "Methexis tests",
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Authority {
    Draft,
    Prospective,
}

#[derive(Deserialize)]
struct CheckReport {
    authority: Authority,
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

    std::io::stdout()
        .write_all(&output.stdout)
        .map_err(|error| format!("cannot forward Methexis validation output: {error}"))?;
    std::io::stderr()
        .write_all(&output.stderr)
        .map_err(|error| format!("cannot forward Methexis validation diagnostics: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "staged Methexis validation failed with {}",
            output.status
        ));
    }

    serde_json::from_slice::<CheckReport>(&output.stdout)
        .map(|report| report.authority)
        .map_err(|error| format!("staged Methexis validation returned an invalid report: {error}"))
}

fn run_cargo(repository: &Path, arguments: &[&str], label: &str) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(arguments)
        .current_dir(repository)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot run {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Authority, CheckReport};

    // Methexis가 실제 prospective authority를 보고한 경우만 activation
    // 구간으로 분류하여, 별도 Git index 사전 읽기와의 불일치가 생기지 않는다.
    #[test]
    fn trusts_only_the_authority_in_the_single_methexis_report() {
        let prospective: CheckReport =
            serde_json::from_str(r#"{"authority":"prospective"}"#).unwrap();
        let ordinary: CheckReport = serde_json::from_str(r#"{"authority":"draft"}"#).unwrap();

        assert_eq!(prospective.authority, Authority::Prospective);
        assert_eq!(ordinary.authority, Authority::Draft);
    }
}
