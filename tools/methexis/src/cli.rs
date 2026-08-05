use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use serde::Serialize;

use crate::{
    CheckClass, check_repository_selected,
    checkpoint::{CheckpointService, StagedTransition},
    context::ContextService,
    review::ReviewService,
};

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis check [--only <class>[,<class>...]]... [--summary] [--unit <id>]
    methexis check --staged-activation
    methexis project-review <request.json>
    methexis build-review <request.json>
    methexis approve <request.json>
    methexis create-checkpoint <request.json>
    methexis propose-activation <request.json>
    methexis refresh-context-manifests <activation-request.json>
    methexis resolve-context <request.json>

COMMANDS:
    check             Validate current SOT integrity or one exact staged activation
    project-review    Write a tracked Korean review Projection
    build-review      Build a local human-review packet
    approve           Record a human-authorized approval proposal
    create-checkpoint Create an immutable trusted-revision Checkpoint proposal
    propose-activation Propose the active Checkpoint with compare-and-swap
    refresh-context-manifests Refresh registered manifests for an activation proposal
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
        [command, request] if command == OsStr::new("refresh-context-manifests") => {
            let root = env::current_dir()?;
            let service = ContextService::new(&root);
            match service.refresh_manifests(Path::new(request)) {
                Ok(result) => write_json(&mut stdout, &result, ExitCode::SUCCESS),
                Err(error) => write_json(&mut stderr, &error, ExitCode::from(2)),
            }
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
    if args == [OsString::from("--staged-activation")] {
        let root = env::current_dir()?;
        let service = CheckpointService::new(&root);
        return match service.check_staged_transition() {
            Ok(StagedTransition::Prospective(report)) => {
                write_json(stdout, &report, ExitCode::SUCCESS)
            },
            Ok(StagedTransition::Ordinary(fallback)) => {
                let report = check_repository_selected(&root, &CheckClass::ALL);
                if let Err(error) = service.finish_staged_fallback(fallback) {
                    write_json(stderr, &error, ExitCode::from(2))
                } else if report.ok {
                    write_json(stdout, &report, ExitCode::SUCCESS)
                } else {
                    write_json(stderr, &report, ExitCode::from(2))
                }
            },
            Err(error) => write_json(stderr, &error, ExitCode::from(2)),
        };
    }
    let selection = match parse_check_selection(args) {
        Ok(selection) => selection,
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
    if selection.unit.is_some()
        && (!selection.summary
            || !selection
                .requested
                .iter()
                .any(|check| matches!(check, CheckClass::Authority | CheckClass::Artifacts)))
    {
        return write_json(
            stderr,
            &CheckArgumentFailure {
                schema: "methexis.error/v1alpha1",
                ok: false,
                error: CheckArgumentError {
                    code: "invalid_check_selector",
                    affected_ids: selection.unit.into_iter().collect(),
                    next_actions: vec![
                        "use --unit with --summary and --only authority or artifacts",
                    ],
                },
            },
            ExitCode::from(2),
        );
    }
    let root = env::current_dir()?;
    let mut report = check_repository_selected(&root, &selection.requested);
    if !report.ok {
        return write_json(stderr, &report, ExitCode::from(2));
    }
    if let Some(unit) = selection.unit.as_deref() {
        report.units.retain(|candidate| candidate.id == unit);
        if report.units.is_empty() {
            return write_json(
                stderr,
                &CheckArgumentFailure {
                    schema: "methexis.error/v1alpha1",
                    ok: false,
                    error: CheckArgumentError {
                        code: "unknown_check_unit",
                        affected_ids: vec![unit.to_owned()],
                        next_actions: vec!["choose an id reported by methexis check"],
                    },
                },
                ExitCode::from(2),
            );
        }
    } else if selection.summary {
        report.units.clear();
    }
    if selection.summary {
        write_json(stdout, &CheckSummary::from(&report), ExitCode::SUCCESS)
    } else {
        write_json(stdout, &report, ExitCode::SUCCESS)
    }
}

struct CheckSelection {
    requested: Vec<CheckClass>,
    summary: bool,
    unit: Option<String>,
}

fn parse_check_selection(args: &[OsString]) -> Result<CheckSelection, ()> {
    let mut summary = false;
    let mut unit = None;

    let mut requested = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].to_str().ok_or(())?;
        if argument == "--summary" {
            summary = true;
            index += 1;
            continue;
        }
        if argument == "--unit" {
            index += 1;
            let value = args.get(index).and_then(|value| value.to_str()).ok_or(())?;
            if value.is_empty() || unit.replace(value.to_owned()).is_some() {
                return Err(());
            }
            index += 1;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--unit=") {
            if value.is_empty() || unit.replace(value.to_owned()).is_some() {
                return Err(());
            }
            index += 1;
            continue;
        }
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
    if requested.is_empty() {
        requested.extend(CheckClass::ALL);
    }
    Ok(CheckSelection {
        requested,
        summary,
        unit,
    })
}

#[derive(Serialize)]
struct CheckSummary<'report> {
    schema: &'static str,
    ok: bool,
    requested_checks: &'report [CheckClass],
    executed_checks: &'report [CheckClass],
    checks: &'report [crate::CheckOutcome],
    authority: &'static str,
    affected_ids: &'report [String],
    units: &'report [crate::UnitRevision],
    diagnostic_count: usize,
}

impl<'report> From<&'report crate::CheckReport> for CheckSummary<'report> {
    fn from(report: &'report crate::CheckReport) -> Self {
        Self {
            schema: "methexis.check-summary/v1alpha1",
            ok: report.ok,
            requested_checks: &report.requested_checks,
            executed_checks: &report.executed_checks,
            checks: &report.checks,
            authority: report.authority,
            affected_ids: &report.affected_ids,
            units: &report.units,
            diagnostic_count: report.diagnostics.len(),
        }
    }
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
