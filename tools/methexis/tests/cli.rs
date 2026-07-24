use std::{
    ffi::OsString,
    io::{self, Write},
    process::Command,
};

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot command shell

USAGE:
    methexis [--help | --version]

No knowledge operations are implemented yet.
",
);

fn methexis() -> Command {
    Command::new(env!("CARGO_BIN_EXE_methexis"))
}

#[test]
fn help_describes_only_the_bootstrap_surface() {
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
