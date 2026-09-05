use std::process::ExitCode;

use crate::{account, command, connection, diagnostic::AppError, local_tools, session, usage};

mod codex_diagnostics;
mod output;
mod runtime;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn write_session_output(output: &str) -> std::io::Result<()> {
    output::write_session_output(output)
}
use codex_diagnostics::{
    CodexWarningCollector, error_after_codex_diagnostics, publish_pending_codex_diagnostics,
};
use output::write_command_output;

pub(super) fn run() -> ExitCode {
    local_tools::initialize_process_file_mode();
    let command = match command::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
            let _ = error.print();
            return ExitCode::from(exit_code);
        },
    };
    match dispatch(command) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let _ = error.print();
            ExitCode::FAILURE
        },
    }
}

fn dispatch(command: command::Command) -> Result<ExitCode, AppError> {
    match command {
        command::Command::Account(command) => {
            let result = account::run(command)?;
            output::finish_account_output(
                result,
                write_command_output,
                output::write_cli_diagnostics,
            )
            .map(account_exit_code)
        },
        command::Command::Connect(command) => success_exit(run_connect_command(command)),
        command::Command::Default(command) => {
            success_exit(write_command_output(connection::run_default(command)?))
        },
        command::Command::Model(command) => success_exit(write_command_output(
            connection::run_model_activation(command)?,
        )),
        command::Command::Disconnect(command) => {
            success_exit(write_command_output(connection::run_disconnect(command)?))
        },
        command::Command::Session(command) => success_exit(run_session_command(command)),
        command::Command::Usage(command) => {
            success_exit(output::write_session_command_output(usage::run(command)?))
        },
        command::Command::Live(options) => success_exit(runtime::run_live_session(options)),
        command::Command::Print(options) => success_exit(runtime::run_print_session(options)),
    }
}

fn account_exit_code(completion: account::AccountCompletion) -> ExitCode {
    match completion {
        account::AccountCompletion::Success => ExitCode::SUCCESS,
        account::AccountCompletion::RefreshFailures => ExitCode::FAILURE,
    }
}

fn success_exit(result: Result<(), AppError>) -> Result<ExitCode, AppError> {
    result.map(|()| ExitCode::SUCCESS)
}

fn run_connect_command(command: command::ConnectCommand) -> Result<(), AppError> {
    let codex_warnings = CodexWarningCollector::default();
    match connection::run_connect_with_codex_warning_observer(
        command,
        Some(codex_warnings.observer()),
    ) {
        Ok(output) => {
            write_command_output(output)?;
            publish_pending_codex_diagnostics(&codex_warnings)
        },
        Err(error) => Err(error_after_codex_diagnostics(error, &codex_warnings)),
    }
}

fn run_session_command(command: command::SessionCommand) -> Result<(), AppError> {
    let output = session::run(command)?;
    output::write_session_command_output(output)
}
