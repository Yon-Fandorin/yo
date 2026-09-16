use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    process::ExitCode,
};

use serde::Serialize;

use super::{
    errors::{ArgumentError, ArgumentFailure, argument_failure},
    output::write_json,
};
use crate::{
    CheckClass, CheckOutcome, CheckReport, UnitRevision, check_repository_selected,
    checkpoint::{CheckpointService, StagedTransition},
};

pub(super) fn run_check(
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
                &argument_failure("invalid_check_selector", Vec::new()),
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
            &ArgumentFailure {
                schema: "methexis.error/v1alpha1",
                ok: false,
                error: ArgumentError {
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
                &ArgumentFailure {
                    schema: "methexis.error/v1alpha1",
                    ok: false,
                    error: ArgumentError {
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
    checks: &'report [CheckOutcome],
    authority: &'static str,
    affected_ids: &'report [String],
    units: &'report [UnitRevision],
    diagnostic_count: usize,
}

impl<'report> From<&'report CheckReport> for CheckSummary<'report> {
    fn from(report: &'report CheckReport) -> Self {
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
