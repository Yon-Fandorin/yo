use std::{
    env::current_dir,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use super::output::write_json;
use crate::author::AuthorService;

pub(super) fn run_author_operation(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = current_dir()?;
    let service = AuthorService::new(&root);
    match service.author_revision(Path::new(request)) {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}
