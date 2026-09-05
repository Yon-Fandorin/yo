use std::io::Write;

use crate::{
    command,
    interaction::diagnostic::{AppError, CliDiagnostic},
};

pub(super) fn write_command_output(output: String) -> Result<(), AppError> {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| AppError::single("writing command output", error))
}

pub(super) fn finish_account_output(
    result: command::AccountRunOutput,
    publish: impl FnOnce(String) -> Result<(), AppError>,
    publish_diagnostics: impl FnOnce(&[CliDiagnostic]) -> Result<(), AppError>,
) -> Result<command::AccountCompletion, AppError> {
    let command::AccountRunOutput {
        output,
        diagnostics,
        completion,
    } = result;
    finish_command_output(output, &diagnostics, publish, publish_diagnostics)?;
    Ok(completion)
}

fn finish_command_output(
    output: String,
    diagnostics: &[CliDiagnostic],
    publish: impl FnOnce(String) -> Result<(), AppError>,
    publish_diagnostics: impl FnOnce(&[CliDiagnostic]) -> Result<(), AppError>,
) -> Result<(), AppError> {
    publish(output)?;
    publish_diagnostics(diagnostics)
}

pub(super) fn write_cli_diagnostics(diagnostics: &[CliDiagnostic]) -> Result<(), AppError> {
    let mut stderr = std::io::stderr().lock();
    write_cli_diagnostics_to_and_flush(diagnostics, &mut stderr)
}

fn write_cli_diagnostics_to<W: Write>(
    diagnostics: &[CliDiagnostic],
    writer: &mut W,
) -> Result<(), AppError> {
    for diagnostic in diagnostics {
        writeln!(writer, "yo: warning: {}", diagnostic.message())
            .map_err(|error| AppError::single("writing command diagnostic", error))?;
    }
    Ok(())
}

fn write_cli_diagnostics_to_and_flush<W: Write>(
    diagnostics: &[CliDiagnostic],
    writer: &mut W,
) -> Result<(), AppError> {
    write_cli_diagnostics_to(diagnostics, writer)?;
    writer
        .flush()
        .map_err(|error| AppError::single("flushing command diagnostics", error))
}

pub(super) fn write_session_output(output: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()
}

pub(super) fn write_session_command_output(output: command::SessionOutput) -> Result<(), AppError> {
    let command::SessionOutput {
        stdout,
        diagnostics,
    } = output;
    finish_command_output(
        stdout,
        &diagnostics,
        |output| {
            write_session_output(&output)
                .map_err(|error| AppError::single("writing Session command output", error))
        },
        write_cli_diagnostics,
    )
}

#[cfg(test)]
mod tests;
