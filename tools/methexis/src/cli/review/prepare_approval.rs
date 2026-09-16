use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use super::super::{
    errors::argument_failure,
    output::{write_json, write_json_pretty},
};
use crate::review::ReviewService;

pub(in crate::cli) fn run_prepare_approval(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let Ok(parsed) = parse_prepare_approval(args) else {
        return write_json(
            stderr,
            &argument_failure("invalid_prepare_arguments", Vec::new()),
            ExitCode::from(2),
        );
    };
    let root = env::current_dir()?;
    let service = ReviewService::new(&root);
    let result = match &parsed.target {
        PrepareApprovalTarget::Manifest(manifest) => service.prepare_approval(
            Path::new(manifest),
            &parsed.reviewer,
            parsed.replace_current,
        ),
        PrepareApprovalTarget::Canonical {
            knowledge_id,
            revision,
        } => service.prepare_canonical_approval(
            knowledge_id,
            revision,
            &parsed.reviewer,
            parsed.replace_current,
        ),
    };
    match result {
        Ok(request) => write_json_pretty(stdout, &request, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PrepareApprovalTarget {
    Manifest(OsString),
    Canonical {
        knowledge_id: String,
        revision: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
struct PrepareApprovalArgs {
    target: PrepareApprovalTarget,
    reviewer: String,
    replace_current: bool,
}

fn parse_prepare_approval(args: &[OsString]) -> Result<PrepareApprovalArgs, ()> {
    let mut manifest = None;
    let mut canonical = None;
    let mut revision = None;
    let mut reviewer = None;
    let mut replace_current = false;

    let mut index = 0;
    while index < args.len() {
        let argument = args[index].to_str().ok_or(())?;
        if argument == "--replace-current" {
            if replace_current {
                return Err(());
            }
            replace_current = true;
            index += 1;
            continue;
        }
        let (kind, value) =
            if argument == "--reviewer" || argument == "--canonical" || argument == "--revision" {
                index += 1;
                let value = args.get(index).and_then(|value| value.to_str()).ok_or(())?;
                if value.starts_with("--") {
                    return Err(());
                }
                (argument, value)
            } else if let Some(value) = argument.strip_prefix("--reviewer=") {
                ("--reviewer", value)
            } else if let Some(value) = argument.strip_prefix("--canonical=") {
                ("--canonical", value)
            } else if let Some(value) = argument.strip_prefix("--revision=") {
                ("--revision", value)
            } else if argument.starts_with("--") {
                return Err(());
            } else {
                if manifest.replace(args[index].clone()).is_some() {
                    return Err(());
                }
                index += 1;
                continue;
            };
        if value.is_empty() {
            return Err(());
        }
        let slot = match kind {
            "--reviewer" => &mut reviewer,
            "--canonical" => &mut canonical,
            "--revision" => &mut revision,
            _ => unreachable!(),
        };
        if slot.replace(value.to_owned()).is_some() {
            return Err(());
        }
        index += 1;
    }
    let reviewer = reviewer.ok_or(())?;
    let target = match (manifest, canonical, revision) {
        (Some(manifest), None, None) => PrepareApprovalTarget::Manifest(manifest),
        (None, Some(knowledge_id), Some(revision)) => PrepareApprovalTarget::Canonical {
            knowledge_id,
            revision,
        },
        _ => return Err(()),
    };
    Ok(PrepareApprovalArgs {
        target,
        reviewer,
        replace_current,
    })
}

#[cfg(test)]
mod tests;
