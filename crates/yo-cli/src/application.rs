use std::process::ExitCode;

use crate::{command, diagnostic::AppError, local_tools};

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

fn dispatch(command_value: command::Command) -> Result<ExitCode, AppError> {
    match command_value {
        command::Command::Account(account_command) => {
            let result = command::run_account(account_command)?;
            output::finish_account_output(
                result,
                write_command_output,
                output::write_cli_diagnostics,
            )
            .map(account_exit_code)
        },
        command::Command::Connect(connect_command) => {
            success_exit(run_connect_command(connect_command))
        },
        command::Command::Default(default_command) => {
            success_exit(write_command_output(command::run_default(default_command)?))
        },
        command::Command::Model(model_command) => success_exit(write_command_output(
            command::run_model_activation(model_command)?,
        )),
        command::Command::Disconnect(disconnect_command) => success_exit(write_command_output(
            command::run_disconnect(disconnect_command)?,
        )),
        command::Command::Session(session_command) => {
            success_exit(run_session_command(session_command))
        },
        command::Command::Usage(usage_command) => success_exit(
            output::write_session_command_output(command::run_usage(usage_command)?),
        ),
        command::Command::Live(options) => success_exit(runtime::run_live_session(options)),
        command::Command::Print(options) => success_exit(runtime::run_print_session(options)),
    }
}

fn account_exit_code(completion: command::AccountCompletion) -> ExitCode {
    match completion {
        command::AccountCompletion::Success => ExitCode::SUCCESS,
        command::AccountCompletion::RefreshFailures => ExitCode::FAILURE,
    }
}

fn success_exit(result: Result<(), AppError>) -> Result<ExitCode, AppError> {
    result.map(|()| ExitCode::SUCCESS)
}

fn run_connect_command(connect_command: command::ConnectCommand) -> Result<(), AppError> {
    let codex_warnings = CodexWarningCollector::default();
    match command::run_connect(connect_command, Some(codex_warnings.observer())) {
        Ok(output) => {
            write_command_output(output)?;
            publish_pending_codex_diagnostics(&codex_warnings)
        },
        Err(error) => Err(error_after_codex_diagnostics(error, &codex_warnings)),
    }
}

fn run_session_command(session_command: command::SessionCommand) -> Result<(), AppError> {
    let output = command::run_session(session_command)?;
    output::write_session_command_output(output)
}
