use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use super::current_repository;
use crate::slice_contract as workflow;

pub(super) fn run(
    action: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if action == "bind" {
        return run_slice_contract_bind(arguments);
    }
    Err(super::general_usage())
}

fn run_slice_contract_bind(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let contract = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(slice_contract_usage)?;
    if arguments.next().is_some() {
        return Err(slice_contract_usage());
    }
    let repository = current_repository()?;
    workflow::bind(&repository, &contract)
}

fn slice_contract_usage() -> String {
    "usage: cargo xtask slice-contract bind <slice-contract.json>".to_owned()
}
