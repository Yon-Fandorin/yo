use std::{
    ffi::OsString,
    io::{self, Write},
    process::Command,
};

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis check
    methexis project-review <request.json>
    methexis build-review <request.json>
    methexis approve <request.json>
    methexis create-checkpoint <request.json>
    methexis propose-activation <request.json>
    methexis resolve-context <request.json>

COMMANDS:
    check             Validate Draft records and trusted Source-aware eligibility
    project-review    Write a tracked Korean review Projection
    build-review      Build a local human-review packet
    approve           Record a human-authorized approval proposal
    create-checkpoint Create an immutable trusted-revision Checkpoint proposal
    propose-activation Propose the active Checkpoint with compare-and-swap
    resolve-context    Build or reuse deterministic token-bounded agent context

Run commands from the repository root. Mutations remain Draft proposals until
trusted integration. Check derives approval and active/degraded eligibility
from local develop, then uses current Source observations only to demote it.
",
);

fn methexis() -> Command {
    Command::new(env!("CARGO_BIN_EXE_methexis"))
}

#[test]
fn help_describes_the_source_aware_check_surface() {
    let output = methexis().arg("--help").output().expect("run methexis");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("help is UTF-8"),
        HELP,
    );
}

#[test]
fn no_arguments_uses_the_same_bootstrap_help() {
    let output = methexis().output().expect("run methexis");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("help is UTF-8"),
        HELP,
    );
}

#[test]
fn version_uses_the_package_version() {
    let output = methexis().arg("--version").output().expect("run methexis");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version is UTF-8"),
        format!("methexis {}\n", env!("CARGO_PKG_VERSION")),
    );
}

#[test]
fn unsupported_input_is_a_structured_failure() {
    let output = methexis()
        .arg("compile")
        .output()
        .expect("run unsupported methexis command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("error is UTF-8"),
        concat!(
            "{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,",
            "\"error\":{\"code\":\"unsupported_command\",",
            "\"affected_ids\":[],",
            "\"next_actions\":[\"methexis --help\"]}}\n",
        ),
    );
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "injected"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn stream_failures_are_returned_to_the_binary_boundary() {
    let error = methexis::run(Vec::<OsString>::new(), FailingWriter, Vec::<u8>::new())
        .expect_err("injected writer must fail");

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn check_reports_the_repository_corpus_on_stdout() {
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate is under <repository>/tools/methexis");
    let output = methexis()
        .current_dir(repository_root)
        .arg("check")
        .output()
        .expect("run methexis check");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("check output is JSON");
    assert_eq!(report["schema"], "methexis.check/v1alpha1");
    assert_eq!(report["ok"], true);
    assert_eq!(report["authority"], "draft");
    let units = report["units"].as_array().expect("units are an array");
    assert_eq!(
        units
            .iter()
            .map(|unit| unit["id"].as_str().expect("unit has an ID"))
            .collect::<Vec<_>>(),
        [
            "tui.architecture.evidence-based-split",
            "tui.architecture.module-boundaries",
            "tui.crate.ui-only-boundary",
            "tui.dependencies.selection-gate",
            "tui.runtime.typed-flow",
        ]
    );
    assert_eq!(report["diagnostics"].as_array().map(Vec::len), Some(0));
}

#[test]
fn check_reports_validation_failures_on_stderr() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("local-invalid");
    let output = methexis()
        .current_dir(fixture)
        .arg("check")
        .output()
        .expect("run failing methexis check");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("failure output is JSON");
    assert_eq!(report["schema"], "methexis.check/v1alpha1");
    assert_eq!(report["ok"], false);
    assert_eq!(report["authority"], "draft");
    assert_eq!(report["snapshot_revision"], serde_json::Value::Null);
}
