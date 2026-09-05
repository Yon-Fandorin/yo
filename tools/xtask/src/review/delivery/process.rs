use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{DIAGNOSTIC_LIMIT, REVIEW_RESULT_LIMIT, combine_failures, model::ProcessOutcome};
use crate::{
    bounded_file,
    review::egress::{AuthorizedDelivery, AuthorizedHostDelivery},
};

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct ProcessCapture {
    pub(super) status: Option<ExitStatus>,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) failure: Option<String>,
}

struct DeliveryCommand<'a> {
    session_repository: &'a Path,
    arguments: &'a [String],
    execution_isolation: Option<&'a str>,
}

pub(super) fn execute_once(
    yo_binary: &Path,
    integration: &Path,
    output: &Path,
    model: &str,
    delivery: &AuthorizedDelivery,
) -> ProcessCapture {
    execute_once_with_timeout(
        yo_binary,
        integration,
        output,
        model,
        delivery,
        DELIVERY_TIMEOUT,
    )
}

pub(super) fn execute_once_with_timeout(
    yo_binary: &Path,
    integration: &Path,
    output: &Path,
    model: &str,
    delivery: &AuthorizedDelivery,
    timeout: Duration,
) -> ProcessCapture {
    let arguments = vec![
        "-p".to_owned(),
        "--model".to_owned(),
        model.to_owned(),
        "--no-tools".to_owned(),
    ];
    execute_command_once_with_timeout(
        yo_binary,
        integration,
        output,
        DeliveryCommand {
            session_repository: &output.join("sessions"),
            arguments: &arguments,
            execution_isolation: None,
        },
        &delivery.packet_bytes,
        timeout,
    )
}

pub(super) fn execute_continuation_once(
    yo_binary: &Path,
    integration: &Path,
    output: &Path,
    session_repository: &Path,
    session_id: &str,
    delivery: &AuthorizedDelivery,
) -> ProcessCapture {
    let arguments = vec![
        "-p".to_owned(),
        "--resume".to_owned(),
        session_id.to_owned(),
    ];
    execute_command_once_with_timeout(
        yo_binary,
        integration,
        output,
        DeliveryCommand {
            session_repository,
            arguments: &arguments,
            execution_isolation: None,
        },
        &delivery.packet_bytes,
        DELIVERY_TIMEOUT,
    )
}

pub(super) fn execute_delegated_once(
    yo_binary: &Path,
    integration: &Path,
    output: &Path,
    delivery: &AuthorizedHostDelivery,
    execution_isolation: Option<&str>,
) -> ProcessCapture {
    let arguments = vec![
        "-p".to_owned(),
        "--model".to_owned(),
        format!("host:{}", delivery.host),
        "--sandbox".to_owned(),
        "read-only".to_owned(),
    ];
    execute_command_once_with_timeout(
        yo_binary,
        integration,
        output,
        DeliveryCommand {
            session_repository: &output.join("sessions"),
            arguments: &arguments,
            execution_isolation,
        },
        &delivery.packet_bytes,
        DELIVERY_TIMEOUT,
    )
}

pub(super) fn execute_delegated_continuation_once(
    yo_binary: &Path,
    integration: &Path,
    output: &Path,
    session_repository: &Path,
    session_id: &str,
    delivery: &AuthorizedHostDelivery,
    execution_isolation: Option<&str>,
) -> ProcessCapture {
    let arguments = vec![
        "-p".to_owned(),
        "--resume".to_owned(),
        session_id.to_owned(),
    ];
    execute_command_once_with_timeout(
        yo_binary,
        integration,
        output,
        DeliveryCommand {
            session_repository,
            arguments: &arguments,
            execution_isolation,
        },
        &delivery.packet_bytes,
        DELIVERY_TIMEOUT,
    )
}

fn execute_command_once_with_timeout(
    yo_binary: &Path,
    integration: &Path,
    output: &Path,
    delivery_command: DeliveryCommand<'_>,
    packet: &[u8],
    timeout: Duration,
) -> ProcessCapture {
    let DeliveryCommand {
        session_repository,
        arguments,
        execution_isolation,
    } = delivery_command;
    let stdout_path = output.join(".review.stdout.tmp");
    let stderr_path = output.join(".review.stderr.tmp");
    let stdout = match create_new(&stdout_path, "temporary review stdout") {
        Ok(stdout) => stdout,
        Err(error) => return failed_capture(error),
    };
    let stderr = match create_new(&stderr_path, "temporary review stderr") {
        Ok(stderr) => stderr,
        Err(error) => {
            drop(stdout);
            let cleanup = fs::remove_file(&stdout_path).err().map(|cleanup| {
                format!("cannot remove temporary review stdout after setup failure: {cleanup}")
            });
            return failed_capture(
                combine_failures(Some(error), cleanup)
                    .expect("temporary capture setup has at least one failure"),
            );
        },
    };
    let mut command = match execution_isolation {
        None | Some(crate::grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE) => {
            let mut command = Command::new(yo_binary);
            command.args(arguments);
            command
        },
        Some(crate::grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE) => {
            match crate::grok_outer_sandbox::command(
                yo_binary,
                arguments,
                integration,
                session_repository,
            ) {
                Ok(command) => command,
                Err(error) => {
                    drop(stdout);
                    drop(stderr);
                    return finish_capture(
                        &stdout_path,
                        &stderr_path,
                        None,
                        Some(format!(
                            "cannot construct the claimed Grok outer sandbox: {error}"
                        )),
                    );
                },
            }
        },
        Some(other) => {
            drop(stdout);
            drop(stderr);
            return finish_capture(
                &stdout_path,
                &stderr_path,
                None,
                Some(format!(
                    "unsupported delegated review execution isolation `{other}`"
                )),
            );
        },
    };
    let child = command
        .env("YO_SESSION_REPOSITORY", session_repository)
        .current_dir(integration)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn();
    let (status, process_failure) = match child {
        Ok(mut child) => {
            let writer = child.stdin.take().map(|mut stdin| {
                let packet = packet.to_vec();
                thread::Builder::new()
                    .name("review-delivery-stdin".to_owned())
                    .spawn(move || {
                        stdin.write_all(&packet).map_err(|error| {
                            format!(
                                "cannot finish the immutable packet write after claiming delivery: {error}"
                            )
                        })
                    })
            });
            let writer_failure = match writer {
                Some(Ok(writer)) => Some(writer),
                Some(Err(error)) => {
                    let cleanup = terminate_and_reap(&mut child);
                    let failure = combine_failures(
                        Some(format!(
                            "cannot start immutable packet writer after claiming delivery: {error}"
                        )),
                        cleanup,
                    );
                    return finish_capture(&stdout_path, &stderr_path, None, failure);
                },
                None => {
                    let cleanup = terminate_and_reap(&mut child);
                    let failure = combine_failures(
                        Some("claimed yo delivery has no writable stdin".to_owned()),
                        cleanup,
                    );
                    return finish_capture(&stdout_path, &stderr_path, None, failure);
                },
            };
            let (status, wait_failure) = wait_with_timeout(&mut child, timeout);
            let write_failure = writer_failure.and_then(|writer| match writer.join() {
                Ok(result) => result.err(),
                Err(_) => {
                    Some("immutable packet writer panicked after claiming delivery".to_owned())
                },
            });
            (status, combine_failures(write_failure, wait_failure))
        },
        Err(error) => (
            None,
            Some(format!(
                "cannot start the claimed current-develop yo delivery: {error}"
            )),
        ),
    };
    finish_capture(&stdout_path, &stderr_path, status, process_failure)
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> (Option<ExitStatus>, Option<String>) {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (Some(status), None),
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(WAIT_POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
            },
            Ok(None) => {
                let cleanup = terminate_and_reap(child);
                return (
                    None,
                    combine_failures(
                        Some(format!(
                            "claimed current-develop yo delivery exceeded its {}-second deadline",
                            timeout.as_secs()
                        )),
                        cleanup,
                    ),
                );
            },
            Err(error) => {
                let cleanup = terminate_and_reap(child);
                return (
                    None,
                    combine_failures(
                        Some(format!("cannot observe the claimed yo delivery: {error}")),
                        cleanup,
                    ),
                );
            },
        }
    }
}

fn terminate_and_reap(child: &mut std::process::Child) -> Option<String> {
    let kill_failure = child
        .kill()
        .err()
        .map(|error| format!("cannot terminate claimed yo delivery: {error}"));
    let wait_failure = child
        .wait()
        .err()
        .map(|error| format!("cannot reap claimed yo delivery: {error}"));
    combine_failures(kill_failure, wait_failure)
}

fn finish_capture(
    stdout_path: &Path,
    stderr_path: &Path,
    status: Option<ExitStatus>,
    failure: Option<String>,
) -> ProcessCapture {
    let (stdout, stdout_failure) = match bounded_file::read_regular(
        stdout_path,
        REVIEW_RESULT_LIMIT,
        "temporary review stdout",
    ) {
        Ok(stdout) => (stdout, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let (stderr, stderr_failure) = match bounded_file::read_regular(
        stderr_path,
        DIAGNOSTIC_LIMIT,
        "temporary review stderr",
    ) {
        Ok(stderr) => (stderr, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let stdout_cleanup = fs::remove_file(stdout_path)
        .err()
        .map(|error| format!("cannot remove temporary review stdout: {error}"));
    let stderr_cleanup = fs::remove_file(stderr_path)
        .err()
        .map(|error| format!("cannot remove temporary review stderr: {error}"));
    let failure = [
        failure,
        stdout_failure,
        stderr_failure,
        stdout_cleanup,
        stderr_cleanup,
    ]
    .into_iter()
    .fold(None, combine_failures);
    ProcessCapture {
        status,
        stdout,
        stderr,
        failure,
    }
}

fn failed_capture(failure: String) -> ProcessCapture {
    ProcessCapture {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        failure: Some(failure),
    }
}

fn create_new(path: &Path, label: &str) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create {label} {}: {error}", path.display()))
}

pub(super) fn process_outcome(status: Option<&ExitStatus>) -> ProcessOutcome {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    ProcessOutcome {
        exit_code: status.and_then(ExitStatus::code),
        #[cfg(unix)]
        signal: status.and_then(ExitStatusExt::signal),
    }
}

pub(super) fn exit_label(status: &ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    status.code().map_or_else(
        || "unknown status".to_owned(),
        |code| format!("exit {code}"),
    )
}
