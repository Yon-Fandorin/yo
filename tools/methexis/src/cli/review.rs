use std::{
    env::current_dir,
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use super::output::write_json;
use crate::review::ReviewService;

mod prepare_approval;

pub(super) use prepare_approval::run_prepare_approval;

pub(super) enum ReviewOperation {
    Project,
    Build,
    Approve,
}

pub(super) fn run_review_operation(
    operation: ReviewOperation,
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = current_dir()?;
    let service = ReviewService::new(&root);
    let request = Path::new(request);
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
