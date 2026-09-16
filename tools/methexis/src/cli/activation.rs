use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use super::output::{write_json, write_json_pretty};
use crate::checkpoint::CheckpointService;

pub(super) fn run_prepare_activation(
    output: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    match service.prepare_activation(Path::new(output)) {
        Ok(request) => write_json_pretty(stdout, &request, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}
