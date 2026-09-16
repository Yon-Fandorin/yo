use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[cfg(test)]
mod tests;

use super::super::current_repository;
use crate::{
    activation_slice, impact, impact::ImpactInput, slice_accept, slice_close, slice_create,
    slice_gate, slice_status,
};

pub(super) fn run(
    scope: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    match scope {
        value if value == "create" => run_slice_create(arguments),
        value if value == "create-activation" => run_activation_slice(arguments),
        value if value == "close" => run_slice_close(arguments),
        value if value == "gate" => run_slice_gate(arguments),
        value if value == "commit" => run_slice_commit(arguments),
        value if value == "accept" => run_slice_accept(arguments),
        value if value == "status" => run_slice_status(arguments),
        _ => Err(super::super::general_usage()),
    }
}

fn run_slice_create(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let contract = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(slice_create_usage)?;
    if arguments.next().is_some() {
        return Err(slice_create_usage());
    }
    let repository = current_repository()?;
    slice_create::run(&repository, &contract)
}

fn run_slice_accept(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(slice_accept_usage)?;
    if first == "prepare" {
        let request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_accept_usage)?;
        if arguments.next().is_some() {
            return Err(slice_accept_usage());
        }
        let repository = current_repository()?;
        return slice_accept::prepare(&repository, &request);
    }
    let request = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err(slice_accept_usage());
    }
    let repository = current_repository()?;
    slice_accept::accept(&repository, &request)
}

fn run_slice_status(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let slice = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(slice_status_usage)?;
    if arguments.next().is_some() {
        return Err(slice_status_usage());
    }
    let repository = current_repository()?;
    slice_status::run(&repository, &slice)
}

fn run_slice_gate(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(slice_gate_usage)?;
    if first == "prepare" {
        let request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_gate_usage)?;
        let output = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_gate_usage)?;
        if arguments.next().is_some() {
            return Err(slice_gate_usage());
        }
        let repository = current_repository()?;
        return slice_gate::prepare_request(&repository, &request, &output);
    }
    let request = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err(slice_gate_usage());
    }
    let repository = current_repository()?;
    slice_gate::run(&repository, &request)
}

fn run_slice_commit(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(slice_commit_usage)?;
    if first == "prepare" {
        let gate_request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_commit_usage)?;
        let message_source = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_commit_usage)?;
        let output = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_commit_usage)?;
        if arguments.next().is_some() {
            return Err(slice_commit_usage());
        }
        let repository = current_repository()?;
        return slice_accept::prepare_commit_message(
            &repository,
            &gate_request,
            &message_source,
            &output,
        );
    }
    let message = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err(slice_commit_usage());
    }
    let input = ImpactInput::load(message.clone(), None, None, true)?;
    impact::preflight::check(&input)?;
    impact::review_coverage::create_accepted_commit(&input.repository, &message)
}

pub(super) fn run_accepted_commit_message_editor(
    target: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err("invalid internal accepted-commit editor invocation".to_owned());
    }
    impact::review_coverage::copy_accepted_commit_message(&PathBuf::from(target))
}

fn run_activation_slice(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(activation_slice_usage)?;
    if arguments.next().is_some() {
        return Err(activation_slice_usage());
    }
    let repository = current_repository()?;
    activation_slice::run(&repository, &request)
}

fn run_slice_close(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let action = arguments
        .next()
        .ok_or_else(slice_close_usage)?
        .to_string_lossy()
        .into_owned();
    if action == "prepare" {
        let request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_close_usage)?;
        if arguments.next().is_some() {
            return Err(slice_close_usage());
        }
        let repository = current_repository()?;
        return slice_close::prepare_metrics(&repository, &request);
    }
    let value = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(slice_close_usage)?;
    let output = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(slice_close_usage());
    }
    let repository = current_repository()?;
    match action.as_str() {
        "plan" => {
            let slice = value
                .to_str()
                .ok_or_else(|| "Slice name must be valid UTF-8".to_owned())?;
            slice_close::plan(&repository, slice, output.as_deref())
        },
        "apply" if output.is_none() => slice_close::apply(&repository, &value),
        _ => Err(slice_close_usage()),
    }
}

fn slice_create_usage() -> String {
    "usage: cargo xtask slice create <slice-contract.json>".to_owned()
}

fn activation_slice_usage() -> String {
    "usage: cargo xtask slice create-activation <request.json>".to_owned()
}

fn slice_close_usage() -> String {
    "usage: cargo xtask slice close <prepare REQUEST.json|plan SLICE [PLAN.json]|apply PLAN.json>"
        .to_owned()
}

fn slice_gate_usage() -> String {
    "usage: cargo xtask slice gate <request.json>\n       cargo xtask slice gate prepare <prepare.json> <gate.json>".to_owned()
}

fn slice_commit_usage() -> String {
    "usage: cargo xtask slice commit <commit-message-file>\n       cargo xtask slice commit prepare <gate.json> <message-source> <message-out>".to_owned()
}

fn slice_status_usage() -> String {
    "usage: cargo xtask slice status <slice>".to_owned()
}

fn slice_accept_usage() -> String {
    "usage: cargo xtask slice accept <request.json>\n       cargo xtask slice accept prepare <prepare.json>".to_owned()
}
