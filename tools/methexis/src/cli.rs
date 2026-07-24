use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    process::ExitCode,
};

use serde::Serialize;

use crate::check_repository;

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis check

COMMANDS:
    check    Validate the current working-tree Draft corpus

Run `check` from the repository root.
Approval and Checkpoint state are not evaluated yet.
",
);

const UNSUPPORTED_COMMAND: &str = "\
{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,\"error\":{\"code\":\"unsupported_command\",\"affected_ids\":[],\"next_actions\":[\"methexis --help\"]}}
";

/// Runs the current Methexis command surface against explicit streams.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> io::Result<ExitCode> {
    let args = args.into_iter().collect::<Vec<_>>();

    match args.as_slice() {
        [] => write_text(&mut stdout, HELP, ExitCode::SUCCESS),
        [arg] if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            write_text(&mut stdout, HELP, ExitCode::SUCCESS)
        },
        [arg] if arg == OsStr::new("--version") || arg == OsStr::new("-V") => {
            writeln!(stdout, "methexis {}", env!("CARGO_PKG_VERSION"))?;
            Ok(ExitCode::SUCCESS)
        },
        [arg] if arg == OsStr::new("check") => {
            let root = env::current_dir()?;
            let report = check_repository(&root);
            if report.ok {
                write_json(&mut stdout, &report, ExitCode::SUCCESS)
            } else {
                write_json(&mut stderr, &report, ExitCode::from(2))
            }
        },
        _ => write_text(&mut stderr, UNSUPPORTED_COMMAND, ExitCode::from(2)),
    }
}

fn write_text(writer: &mut impl Write, text: &str, exit_code: ExitCode) -> io::Result<ExitCode> {
    writer.write_all(text.as_bytes())?;
    Ok(exit_code)
}

fn write_json(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
) -> io::Result<ExitCode> {
    serde_json::to_writer(&mut *writer, value).map_err(io::Error::other)?;
    writer.write_all(b"\n")?;
    Ok(exit_code)
}
