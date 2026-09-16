use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[cfg(test)]
mod tests;

use super::current_repository;
use crate::docs_translation;

pub(super) fn run(
    action: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if action == "accept-translation" {
        return run_docs_accept_translation(arguments);
    }
    Err(super::general_usage())
}

fn run_docs_accept_translation(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let page = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(docs_accept_translation_usage)?;
    if arguments.next().is_some() {
        return Err(docs_accept_translation_usage());
    }
    let repository = current_repository()?;
    docs_translation::accept(&repository, &page)
}

fn docs_accept_translation_usage() -> String {
    "usage: cargo xtask docs accept-translation <relative-page.md>".to_owned()
}
