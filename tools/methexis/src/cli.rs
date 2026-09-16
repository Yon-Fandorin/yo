use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    process::ExitCode,
};

mod activation;
mod author;
mod check;
mod checkpoint;
mod context;
mod errors;
mod help;
mod output;
mod review;

/// Runs the current Methexis command surface against explicit streams.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> io::Result<ExitCode> {
    let args = args.into_iter().collect::<Vec<_>>();

    if let Some(result) = help::run_bootstrap(&args, &mut stdout) {
        return result;
    }

    run_command(&args, &mut stdout, &mut stderr)
}

fn run_command(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    if args.first().is_some_and(|arg| arg == OsStr::new("check")) {
        return check::run_check(&args[1..], stdout, stderr);
    }

    if args
        .first()
        .is_some_and(|arg| arg == OsStr::new("prepare-approval"))
    {
        return review::run_prepare_approval(&args[1..], stdout, stderr);
    }

    match args {
        [command] if command == OsStr::new("prepare-checkpoint") => {
            checkpoint::run_prepare_checkpoint(stdout, stderr)
        },
        [command, request] if command == OsStr::new("author-revision") => {
            author::run_author_operation(request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("project-review") => {
            review::run_review_operation(review::ReviewOperation::Project, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("build-review") => {
            review::run_review_operation(review::ReviewOperation::Build, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("approve") => {
            review::run_review_operation(review::ReviewOperation::Approve, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("create-checkpoint") => {
            checkpoint::run_checkpoint_operation(
                checkpoint::CheckpointOperation::Create,
                request,
                stdout,
                stderr,
            )
        },
        [command, request] if command == OsStr::new("propose-activation") => {
            checkpoint::run_checkpoint_operation(
                checkpoint::CheckpointOperation::Activate,
                request,
                stdout,
                stderr,
            )
        },
        [command, output] if command == OsStr::new("prepare-activation") => {
            activation::run_prepare_activation(output, stdout, stderr)
        },
        [command, request] if command == OsStr::new("resolve-context") => {
            context::run_context_operation(request, stdout, stderr)
        },
        [command, activation, request]
            if command == OsStr::new("resolve-activation-review-context") =>
        {
            context::run_prospective_context_operation(activation, request, stdout, stderr)
        },
        [command, request, build_id] if command == OsStr::new("verify-context-build") => {
            context::run_context_verification(request, build_id, stdout, stderr)
        },
        [command, request] if command == OsStr::new("refresh-context-manifests") => {
            context::run_refresh_context_manifests(request, stdout, stderr)
        },
        _ => output::write_text(stderr, errors::UNSUPPORTED_COMMAND, ExitCode::from(2)),
    }
}
