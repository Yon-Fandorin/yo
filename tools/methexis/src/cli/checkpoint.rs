use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use super::output::{write_json, write_json_pretty};
use crate::checkpoint::CheckpointService;

pub(super) fn run_prepare_checkpoint(
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    match service.prepare_checkpoint() {
        Ok(request) => write_json_pretty(stdout, &request, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

pub(super) enum CheckpointOperation {
    Create,
    Activate,
}

pub(super) fn run_checkpoint_operation(
    operation: CheckpointOperation,
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    let request = Path::new(request);
    let result = match operation {
        CheckpointOperation::Create => service.create(request),
        CheckpointOperation::Activate => service.propose_activation(request),
    };
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}
