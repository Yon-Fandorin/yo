use std::path::PathBuf;

use clap::{ArgGroup, Args};

mod external;
mod import;
mod input;
mod local;
mod picker;
mod presentation;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
#[command(group(
    ArgGroup::new("connection_source")
        .required(true)
        .multiple(false)
        .args(["target", "from"])
))]
pub(super) struct Arguments {
    /// Exact target to connect.
    #[arg(value_name = "TARGET", allow_hyphen_values = true)]
    target: Option<String>,

    /// Import one grouped YAML definition from an absolute path, or from stdin with '-'.
    #[arg(long, value_name = "PATH", allow_hyphen_values = true)]
    from: Option<PathBuf>,

    /// Show the exact connection profile in the confirmation.
    #[arg(short, long)]
    verbose: bool,

    /// Read the external API key from this owner-only file.
    #[arg(long, value_name = "PATH", requires = "yes")]
    credential_file: Option<PathBuf>,

    /// Apply the captured external connection plan without an interactive confirmation.
    #[arg(long, requires = "credential_file", conflicts_with = "verbose")]
    yes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) target: String,
    pub(crate) from: Option<PathBuf>,
    pub(crate) verbose: bool,
    pub(crate) credential_file: Option<PathBuf>,
    pub(crate) yes: bool,
}

impl Arguments {
    pub(super) fn into_command(
        self,
        output: super::output::OutputOptions,
    ) -> Result<Command, clap::Error> {
        super::output::validate(output, "connect", false, false)?;
        Ok(Command {
            target: self.target.unwrap_or_default(),
            from: self.from,
            verbose: self.verbose,
            credential_file: self.credential_file,
            yes: self.yes,
        })
    }
}

pub(crate) fn run(
    command: Command,
    warning_observer: Option<yo_backend_delegated_codex::CodexWarningObserver>,
) -> Result<String, crate::AppError> {
    let config_path = crate::state::connection::absolute_config_path(
        crate::state::config::selected_path()
            .map_err(|error| crate::AppError::single("locating Yo configuration", error))?,
    )?;
    if command.from.is_some() {
        return external::run_definition_import(&config_path, command);
    }
    let host = yo_core::HostId::from_reference(&command.target)
        .map_err(|error| crate::AppError::single("parsing the agent host target", error))?;
    if host.is_some() {
        local::validate_options(&command)?;
    }
    let Some(host) = host else {
        return external::run_external_connect(&config_path, command);
    };
    local::run(&config_path, command, host, warning_observer)
}
