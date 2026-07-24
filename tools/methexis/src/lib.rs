//! Bootstrap shell for the Methexis SOT Pilot in `yo`.
//!
//! Knowledge, approval, Checkpoint, and projection behavior is intentionally
//! absent until its owning Slices are accepted.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    process::ExitCode,
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

const UNSUPPORTED_COMMAND: &str = "\
{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,\"error\":{\"code\":\"unsupported_command\",\"affected_ids\":[],\"next_actions\":[\"methexis --help\"]}}
";

/// Runs the bootstrap command shell against explicit streams.
///
/// Only help and version discovery are supported. All domain operations remain
/// unavailable until later Slices add their contracts and tests.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> io::Result<ExitCode> {
    let args = args.into_iter().collect::<Vec<_>>();

    match args.as_slice() {
        [] => {
            stdout.write_all(HELP.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        },
        [arg] if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            stdout.write_all(HELP.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        },
        [arg] if arg == OsStr::new("--version") || arg == OsStr::new("-V") => {
            writeln!(stdout, "methexis {}", env!("CARGO_PKG_VERSION"))?;
            Ok(ExitCode::SUCCESS)
        },
        _ => {
            stderr.write_all(UNSUPPORTED_COMMAND.as_bytes())?;
            Ok(ExitCode::from(2))
        },
    }
}
