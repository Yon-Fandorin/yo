use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    process::ExitCode,
};

use serde::Serialize;

use crate::{check_repository, review::ReviewService};

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

COMMANDS:
    check             Validate Draft knowledge and approval proposals
    project-review    Write a tracked Korean review Projection
    build-review      Build a local human-review packet
    approve           Record a human-authorized approval proposal

Run commands from the repository root. Mutations remain Draft proposals until
trusted integration. Checkpoint and Source freshness are not evaluated yet.
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
        [command, request] if command == OsStr::new("project-review") => {
            run_operation(ReviewOperation::Project, request, &mut stdout, &mut stderr)
        },
        [command, request] if command == OsStr::new("build-review") => {
            run_operation(ReviewOperation::Build, request, &mut stdout, &mut stderr)
        },
        [command, request] if command == OsStr::new("approve") => {
            run_operation(ReviewOperation::Approve, request, &mut stdout, &mut stderr)
        },
        _ => write_text(&mut stderr, UNSUPPORTED_COMMAND, ExitCode::from(2)),
    }
}

enum ReviewOperation {
    Project,
    Build,
    Approve,
}

fn run_operation(
    operation: ReviewOperation,
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = ReviewService::new(&root);
    let request = std::path::Path::new(request);
    let result = match operation {
        ReviewOperation::Project => service.generate_projection(request),
        ReviewOperation::Build => service.build_review(request),
        ReviewOperation::Approve => service.record_approval(request),
    };
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
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
