use std::{
    env,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use super::{errors::argument_failure, output::write_json};
use crate::context::ContextService;

pub(super) fn run_context_operation(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let result = ContextService::new(&root).resolve(Path::new(request));
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

pub(super) fn run_prospective_context_operation(
    activation_request: &OsStr,
    context_request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let result = ContextService::new(&root)
        .resolve_activation_review(Path::new(activation_request), Path::new(context_request));
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

pub(super) fn run_context_verification(
    request: &OsStr,
    build_id: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let Some(build_id) = build_id.to_str() else {
        return write_json(
            stderr,
            &argument_failure("invalid_verify_arguments", Vec::new()),
            ExitCode::from(2),
        );
    };
    let root = env::current_dir()?;
    let result = ContextService::new(&root).verify(Path::new(request), build_id);
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

pub(super) fn run_refresh_context_manifests(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = ContextService::new(&root);
    match service.refresh_manifests(Path::new(request)) {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}
