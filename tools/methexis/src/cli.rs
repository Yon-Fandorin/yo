use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    process::ExitCode,
};

use serde::Serialize;

use crate::{
    CheckClass, check_repository_selected, checkpoint::CheckpointService, context::ContextService,
    review::ReviewService,
};

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis check [--only <class>[,<class>...]]...
    methexis project-review <request.json>
    methexis build-review <request.json>
    methexis approve <request.json>
    methexis create-checkpoint <request.json>
    methexis propose-activation <request.json>
    methexis resolve-context <request.json>

COMMANDS:
    check             Validate SOT integrity; classes: records, relations, authority, artifacts
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

    if args.first().is_some_and(|arg| arg == OsStr::new("check")) {
        return run_check(&args[1..], &mut stdout, &mut stderr);
    }

    match args.as_slice() {
        [] => write_text(&mut stdout, HELP, ExitCode::SUCCESS),
        [arg] if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            write_text(&mut stdout, HELP, ExitCode::SUCCESS)
        },
        [arg] if arg == OsStr::new("--version") || arg == OsStr::new("-V") => {
            writeln!(stdout, "methexis {}", env!("CARGO_PKG_VERSION"))?;
            Ok(ExitCode::SUCCESS)
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
        [command, request] if command == OsStr::new("create-checkpoint") => {
            run_checkpoint_operation(
                CheckpointOperation::Create,
                request,
                &mut stdout,
                &mut stderr,
            )
        },
        [command, request] if command == OsStr::new("propose-activation") => {
            run_checkpoint_operation(
                CheckpointOperation::Activate,
                request,
                &mut stdout,
                &mut stderr,
            )
        },
        [command, request] if command == OsStr::new("resolve-context") => {
            run_context_operation(request, &mut stdout, &mut stderr)
        },
        _ => write_text(&mut stderr, UNSUPPORTED_COMMAND, ExitCode::from(2)),
    }
}

#[derive(Serialize)]
struct CheckArgumentFailure {
    schema: &'static str,
    ok: bool,
    error: CheckArgumentError,
}

#[derive(Serialize)]
struct CheckArgumentError {
    code: &'static str,
    affected_ids: Vec<String>,
    next_actions: Vec<&'static str>,
}

fn run_check(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let requested = match parse_check_selection(args) {
        Ok(requested) => requested,
        Err(()) => {
            return write_json(
                stderr,
                &CheckArgumentFailure {
                    schema: "methexis.error/v1alpha1",
                    ok: false,
                    error: CheckArgumentError {
                        code: "invalid_check_selector",
                        affected_ids: Vec::new(),
                        next_actions: vec!["methexis --help"],
                    },
                },
                ExitCode::from(2),
            );
        },
    };
    let root = env::current_dir()?;
    let report = check_repository_selected(&root, &requested);
    if report.ok {
        write_json(stdout, &report, ExitCode::SUCCESS)
    } else {
        write_json(stderr, &report, ExitCode::from(2))
    }
}

fn parse_check_selection(args: &[OsString]) -> Result<Vec<CheckClass>, ()> {
    if args.is_empty() {
        return Ok(CheckClass::ALL.to_vec());
    }

    let mut requested = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].to_str().ok_or(())?;
        let value = if argument == "--only" {
            index += 1;
            args.get(index).and_then(|value| value.to_str()).ok_or(())?
        } else if let Some(value) = argument.strip_prefix("--only=") {
            value
        } else {
            return Err(());
        };
        for selector in value.split(',') {
            let selector = selector.trim();
            if selector.is_empty() {
                return Err(());
            }
            requested.push(CheckClass::parse(selector).ok_or(())?);
        }
        index += 1;
    }
    Ok(requested)
}

fn run_context_operation(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let result = ContextService::new(&root).resolve(std::path::Path::new(request));
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

enum CheckpointOperation {
    Create,
    Activate,
}

fn run_checkpoint_operation(
    operation: CheckpointOperation,
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    let request = std::path::Path::new(request);
    let result = match operation {
        CheckpointOperation::Create => service.create(request),
        CheckpointOperation::Activate => service.propose_activation(request),
    };
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
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
