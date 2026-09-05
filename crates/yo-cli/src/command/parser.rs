use std::ffi::OsString;

use clap::{CommandFactory, Parser, Subcommand};
use yo_tui::PresentationMode;

use super::{
    Command, account, connect, default, disconnect, live, model,
    output::{OutputFormat, OutputOptions},
    print, session, usage,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "yo",
    version,
    about = "Agentic coding interface",
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Print one final response and exit without opening the terminal UI.
    #[arg(
        short = 'p',
        long = "print",
        conflicts_with_all = [
            "inline",
            "fullscreen",
            "ascii",
            "continue_session"
        ]
    )]
    print: bool,

    /// Prompt for one print-mode Submission.
    #[arg(value_name = "PROMPT", requires = "print")]
    prompt: Option<String>,

    /// Render inside the current terminal screen.
    #[arg(long, conflicts_with = "fullscreen")]
    inline: bool,

    /// Take over the terminal screen while the session is active.
    #[arg(long, conflicts_with = "inline")]
    fullscreen: bool,

    /// Use ASCII characters instead of rich terminal glyphs.
    #[arg(long, global = true)]
    ascii: bool,

    /// Select human-readable text or a supported machine-readable format.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    /// Resume a Session by its ID.
    #[arg(long, value_name = "SESSION_ID", conflicts_with = "continue_session")]
    resume: Option<yo_core::SessionId>,

    /// Continue the most recent Session in the current workspace.
    #[arg(long = "continue", conflicts_with = "resume")]
    continue_session: bool,

    /// Select a startup target, such as host:grok or provider:account:model.
    #[arg(long, value_name = "MODEL_REFERENCE", allow_hyphen_values = true)]
    model: Option<String>,

    /// Start a native Session without exposing local tools to the model.
    #[arg(long, conflicts_with_all = ["resume", "continue_session"])]
    no_tools: bool,

    /// Restrict a print-mode delegated host Session to read-only review.
    #[arg(
        long,
        value_enum,
        value_name = "MODE",
        requires = "print",
        conflicts_with_all = ["no_tools", "resume", "continue_session"]
    )]
    sandbox: Option<live::SandboxMode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
enum CliCommand {
    /// Show cached or refreshed capacity for one or more account sources.
    Account(account::Arguments),

    /// Connect one service target.
    Connect(connect::Arguments),

    /// Remove one stored external model connection.
    Disconnect(disconnect::Arguments),

    /// Show or change the stored startup default.
    Default(default::Arguments),

    /// Enable or disable one stored model binding for new work.
    Model(model::Arguments),

    /// List stored Sessions or inspect one Session.
    Session(session::Arguments),

    /// Show usage for one stored Session.
    Usage(usage::Arguments),
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, clap::Error> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let subcommands = top_level_subcommand_inventory();
    reject_print_subcommand_overlap(&arguments, &subcommands)?;
    let arguments = normalize_global_output_options(arguments, &subcommands);
    let cli = Cli::try_parse_from(std::iter::once(OsString::from("yo")).chain(arguments))?;
    let output = OutputOptions::from_cli(cli.format, cli.ascii);

    match cli.command {
        Some(CliCommand::Account(arguments)) => {
            arguments.into_command(output).map(Command::Account)
        },
        Some(CliCommand::Connect(arguments)) => {
            arguments.into_command(output).map(Command::Connect)
        },
        Some(CliCommand::Disconnect(arguments)) => {
            arguments.into_command(output).map(Command::Disconnect)
        },
        Some(CliCommand::Default(arguments)) => {
            arguments.into_command(output).map(Command::Default)
        },
        Some(CliCommand::Model(arguments)) => arguments.into_command(output).map(Command::Model),
        Some(CliCommand::Session(arguments)) => {
            arguments.into_command(output).map(Command::Session)
        },
        Some(CliCommand::Usage(arguments)) => arguments.into_command(output).map(Command::Usage),
        None if cli.print => print::from_cli(
            cli.prompt,
            cli.resume,
            cli.model,
            cli.no_tools,
            cli.sandbox,
            output,
        )
        .map(Command::Print),
        None => {
            let mode = match (cli.inline, cli.fullscreen) {
                (false, true) => PresentationMode::Fullscreen,
                (_, false) => PresentationMode::Inline,
                (true, true) => unreachable!("clap rejects conflicting presentation modes"),
            };
            live::from_cli(
                mode,
                output,
                cli.resume,
                cli.continue_session,
                cli.model,
                cli.no_tools,
                cli.sandbox,
            )
            .map(Command::Live)
        },
    }
}

fn normalize_global_output_options(
    arguments: Vec<OsString>,
    subcommands: &[String],
) -> Vec<OsString> {
    let Some(command_index) = top_level_subcommand_index(&arguments, subcommands) else {
        return arguments;
    };
    if command_index == 0 {
        return arguments;
    }

    let mut command_arguments = arguments[command_index..].to_vec();
    let mut global_output_options = Vec::new();
    let mut root_arguments = Vec::new();
    let mut index = 0;
    while index < command_index {
        let argument = &arguments[index];
        if argument == "--ascii" {
            global_output_options.push(argument.clone());
        } else if argument == "--format" {
            global_output_options.push(argument.clone());
            if let Some(value) = arguments.get(index + 1) {
                global_output_options.push(value.clone());
                index += 1;
            }
        } else if argument
            .to_str()
            .is_some_and(|value| value.starts_with("--format="))
        {
            global_output_options.push(argument.clone());
        } else {
            root_arguments.push(argument.clone());
        }
        index += 1;
    }
    if global_output_options.is_empty() {
        return arguments;
    }
    command_arguments.extend(global_output_options);
    root_arguments.extend(command_arguments);
    root_arguments
}

fn top_level_subcommand_index(arguments: &[OsString], subcommands: &[String]) -> Option<usize> {
    let mut skip_option_value = false;
    for (index, argument) in arguments.iter().enumerate() {
        if skip_option_value {
            skip_option_value = false;
            continue;
        }
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if argument == "--" {
            return None;
        }
        if matches!(argument, "--model" | "--resume" | "--sandbox" | "--format") {
            skip_option_value = true;
            continue;
        }
        if subcommands.iter().any(|subcommand| subcommand == argument) {
            return Some(index);
        }
    }
    None
}

fn reject_print_subcommand_overlap(
    arguments: &[OsString],
    subcommands: &[String],
) -> Result<(), clap::Error> {
    let mut print_requested = false;
    let mut subcommand = None;
    let mut skip_option_value = false;
    for argument in arguments {
        if skip_option_value {
            skip_option_value = false;
            continue;
        }
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if argument == "--" {
            break;
        }
        if matches!(argument, "--model" | "--resume" | "--sandbox" | "--format") {
            skip_option_value = true;
            continue;
        }
        if matches!(argument, "-p" | "--print") {
            print_requested = true;
        } else if subcommand.is_none()
            && subcommands.iter().any(|subcommand| subcommand == argument)
        {
            subcommand = Some(argument);
        }
    }
    if let (true, Some(subcommand)) = (print_requested, subcommand) {
        Err(super::raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            format!(
                "-p/--print cannot be combined with the `{subcommand}` subcommand; use `--` before a literal prompt named `{subcommand}`"
            ),
        ))
    } else {
        Ok(())
    }
}

fn top_level_subcommand_inventory() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .flat_map(|subcommand| {
            std::iter::once(subcommand.get_name())
                .chain(subcommand.get_all_aliases())
                .map(str::to_owned)
        })
        .collect()
}
