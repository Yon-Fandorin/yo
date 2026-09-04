use std::{ffi::OsString, num::NonZeroUsize, path::PathBuf};

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use yo_tui::{GlyphProfile, PresentationMode};

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
    sandbox: Option<SandboxMode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
enum CliCommand {
    /// Show cached or refreshed capacity for one or more account sources.
    Account(AccountArguments),

    /// Connect one service target.
    Connect(ConnectArguments),

    /// Remove one stored external model connection.
    Disconnect(DisconnectArguments),

    /// Show or change the stored startup default.
    Default(DefaultArguments),

    /// Enable or disable one stored model binding for new work.
    Model(ModelArguments),

    /// List stored Sessions or inspect one Session.
    Session(SessionArguments),

    /// Show usage for one stored Session.
    Usage(UsageArguments),
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct AccountArguments {
    /// Optional account source: a Provider, or an exact Provider:Account pair.
    #[arg(value_name = "SOURCE")]
    source: Option<String>,

    /// Re-observe the selected account source before displaying it.
    #[arg(long)]
    refresh: bool,

    /// Show the full multi-line capacity details instead of the account table (text only).
    #[arg(long)]
    detail: bool,
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
#[command(group(
    ArgGroup::new("connection_source")
        .required(true)
        .multiple(false)
        .args(["target", "from"])
))]
struct ConnectArguments {
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

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct DisconnectArguments {
    /// Provider whose stored model connection should be removed.
    #[arg(value_name = "PROVIDER", allow_hyphen_values = true)]
    provider: Option<String>,

    /// Exact Account within the Provider.
    #[arg(long, value_name = "ACCOUNT", requires = "provider")]
    account: Option<String>,

    /// Apply the captured unambiguous plan without an interactive confirmation.
    #[arg(long, requires_all = ["provider", "account"])]
    yes: bool,

    /// Show provenance, the exact profile, and remaining models in the confirmation.
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
#[group(required = true, multiple = false)]
struct DefaultArguments {
    /// Exact admitted startup target to persist.
    #[arg(value_name = "TARGET", allow_hyphen_values = true)]
    target: Option<String>,

    /// Clear the stored startup default.
    #[arg(long)]
    unset: bool,
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct ModelArguments {
    #[command(subcommand)]
    action: ModelActionArguments,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
enum ModelActionArguments {
    /// Admit one stored model binding for new work.
    Enable(ModelTargetArguments),
    /// Reject one stored model binding for new work while preserving it for later use.
    Disable(ModelTargetArguments),
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct ModelTargetArguments {
    /// Exact stored model target to mutate.
    #[arg(value_name = "TARGET", allow_hyphen_values = true)]
    target: String,
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct SessionArguments {
    /// Session to inspect.
    #[arg(value_name = "SESSION_ID", conflicts_with_all = ["all", "details"])]
    session_id: Option<yo_core::SessionId>,

    /// Include Sessions outside the current workspace.
    #[arg(long, conflicts_with = "session_id")]
    all: bool,

    /// Include stored metadata in the Session list.
    #[arg(long, conflicts_with = "session_id")]
    details: bool,

    /// Select the stored Session projection.
    #[arg(long, value_enum, value_name = "VIEW", requires = "session_id")]
    view: Option<SessionView>,

    /// Show at most the newest N semantic Transcript records.
    #[arg(long, value_name = "N", requires = "session_id")]
    limit: Option<NonZeroUsize>,

    /// Select how much record payload the Transcript exposes.
    #[arg(long, value_enum, value_name = "MODE", requires = "session_id")]
    content: Option<SessionContent>,
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct UsageArguments {
    /// Session whose usage should be shown.
    #[arg(value_name = "SESSION_ID")]
    session_id: yo_core::SessionId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiveOptions {
    pub(crate) mode: PresentationMode,
    pub(crate) glyph_profile: GlyphProfile,
    pub(crate) selection: LiveSelection,
    pub(crate) model: Option<String>,
    pub(crate) no_tools: bool,
    pub(crate) sandbox: Option<SandboxMode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrintOptions {
    pub(crate) prompt: Option<String>,
    pub(crate) selection: LiveSelection,
    pub(crate) model: Option<String>,
    pub(crate) no_tools: bool,
    pub(crate) sandbox: Option<SandboxMode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SandboxMode {
    ReadOnly,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LiveSelection {
    #[default]
    New,
    Resume(yo_core::SessionId),
    Continue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Account(AccountCommand),
    Connect(ConnectCommand),
    Disconnect(DisconnectCommand),
    Default(DefaultCommand),
    Model(ModelCommand),
    Live(LiveOptions),
    Print(PrintOptions),
    Session(SessionCommand),
    Usage(UsageCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccountCommand {
    pub(crate) source: Option<String>,
    pub(crate) refresh: bool,
    pub(crate) detail: bool,
    pub(crate) output: OutputOptions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputOptions {
    pub(crate) format: OutputFormat,
    pub(crate) glyph_profile: GlyphProfile,
}

impl OutputOptions {
    fn from_cli(format: OutputFormat, ascii: bool) -> Self {
        Self {
            format,
            glyph_profile: glyph_profile(ascii),
        }
    }
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self::from_cli(OutputFormat::Text, false)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConnectCommand {
    pub(crate) target: String,
    pub(crate) from: Option<PathBuf>,
    pub(crate) verbose: bool,
    pub(crate) credential_file: Option<PathBuf>,
    pub(crate) yes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DisconnectCommand {
    pub(crate) provider: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) yes: bool,
    pub(crate) verbose: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DefaultCommand {
    pub(crate) target: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelCommand {
    pub(crate) target: String,
    pub(crate) enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SessionView {
    Chat,
    Transcript,
    Request,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SessionContent {
    None,
    Preview,
    Full,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionCommand {
    pub(crate) session_id: Option<yo_core::SessionId>,
    pub(crate) all: bool,
    pub(crate) details: bool,
    pub(crate) view: SessionView,
    pub(crate) output: OutputOptions,
    pub(crate) limit: Option<NonZeroUsize>,
    pub(crate) content: Option<SessionContent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UsageCommand {
    pub(crate) session_id: yo_core::SessionId,
    pub(crate) output: OutputOptions,
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, clap::Error> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    reject_print_subcommand_overlap(&arguments)?;
    let arguments = normalize_global_output_options(arguments);
    let cli = Cli::try_parse_from(std::iter::once(OsString::from("yo")).chain(arguments))?;
    let output = OutputOptions::from_cli(cli.format, cli.ascii);

    match cli.command {
        Some(CliCommand::Account(arguments)) => {
            validate_output(output, "account", true, true)?;
            Ok(Command::Account(AccountCommand {
                source: arguments.source,
                refresh: arguments.refresh,
                detail: arguments.detail,
                output,
            }))
        },
        Some(CliCommand::Connect(arguments)) => {
            validate_output(output, "connect", false, false)?;
            Ok(Command::Connect(ConnectCommand {
                target: arguments.target.unwrap_or_default(),
                from: arguments.from,
                verbose: arguments.verbose,
                credential_file: arguments.credential_file,
                yes: arguments.yes,
            }))
        },
        Some(CliCommand::Disconnect(arguments)) => {
            validate_output(output, "disconnect", false, false)?;
            Ok(Command::Disconnect(DisconnectCommand {
                provider: arguments.provider,
                account: arguments.account,
                yes: arguments.yes,
                verbose: arguments.verbose,
            }))
        },
        Some(CliCommand::Default(arguments)) => {
            validate_output(output, "default", false, false)?;
            Ok(Command::Default(DefaultCommand {
                target: arguments.target,
            }))
        },
        Some(CliCommand::Model(arguments)) => {
            validate_output(output, "model", false, false)?;
            let (target, enabled) = match arguments.action {
                ModelActionArguments::Enable(arguments) => (arguments.target, true),
                ModelActionArguments::Disable(arguments) => (arguments.target, false),
            };
            Ok(Command::Model(ModelCommand { target, enabled }))
        },
        Some(CliCommand::Session(arguments)) => {
            let view = arguments.view.unwrap_or(SessionView::Chat);
            if view != SessionView::Transcript
                && (arguments.limit.is_some() || arguments.content.is_some())
            {
                return Err(raw_command_error(
                    clap::error::ErrorKind::ArgumentConflict,
                    "--limit and --content are supported only with --view transcript",
                ));
            }
            validate_output(
                output,
                if arguments.session_id.is_some() {
                    "session"
                } else {
                    "session list"
                },
                false,
                arguments.session_id.is_some(),
            )?;
            Ok(Command::Session(SessionCommand {
                session_id: arguments.session_id,
                all: arguments.all,
                details: arguments.details,
                view,
                output,
                limit: arguments.limit,
                content: arguments.content,
            }))
        },
        Some(CliCommand::Usage(arguments)) => {
            validate_output(output, "usage", false, true)?;
            Ok(Command::Usage(UsageCommand {
                session_id: arguments.session_id,
                output,
            }))
        },
        None if cli.print => {
            validate_output(output, "print", false, false)?;
            let selection = cli.resume.map_or(LiveSelection::New, LiveSelection::Resume);
            if selection != LiveSelection::New && cli.model.is_some() {
                return Err(raw_command_error(
                    clap::error::ErrorKind::ArgumentConflict,
                    "--print --resume uses the stored model binding and cannot be combined with --model",
                ));
            }
            Ok(Command::Print(PrintOptions {
                prompt: cli.prompt,
                selection,
                model: cli.model,
                no_tools: cli.no_tools,
                sandbox: cli.sandbox,
            }))
        },
        None => {
            validate_output(output, "live", false, true)?;
            Ok(Command::Live(LiveOptions {
                mode: match (cli.inline, cli.fullscreen) {
                    (false, true) => PresentationMode::Fullscreen,
                    (_, false) => PresentationMode::Inline,
                    (true, true) => unreachable!("clap rejects conflicting presentation modes"),
                },
                glyph_profile: output.glyph_profile,
                selection: match (cli.resume, cli.continue_session) {
                    (Some(session_id), false) => LiveSelection::Resume(session_id),
                    (None, true) => LiveSelection::Continue,
                    (None, false) => LiveSelection::New,
                    (Some(_), true) => {
                        unreachable!("clap rejects conflicting continuation options")
                    },
                },
                model: cli.model,
                no_tools: cli.no_tools,
                sandbox: cli.sandbox,
            }))
        },
    }
}

fn validate_output(
    output: OutputOptions,
    command: &str,
    supports_json: bool,
    supports_ascii: bool,
) -> Result<(), clap::Error> {
    if output.format == OutputFormat::Json && !supports_json {
        return Err(raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            format!("--format json is not supported by `{command}`"),
        ));
    }
    if output.glyph_profile == GlyphProfile::Ascii && !supports_ascii {
        return Err(raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            format!("--ascii is not supported by `{command}`"),
        ));
    }
    Ok(())
}

fn normalize_global_output_options(arguments: Vec<OsString>) -> Vec<OsString> {
    let Some(command_index) = top_level_subcommand_index(&arguments) else {
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

fn top_level_subcommand_index(arguments: &[OsString]) -> Option<usize> {
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
        if matches!(
            argument,
            "account" | "connect" | "disconnect" | "default" | "model" | "session" | "usage"
        ) {
            return Some(index);
        }
    }
    None
}

fn reject_print_subcommand_overlap(arguments: &[OsString]) -> Result<(), clap::Error> {
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
            && matches!(
                argument,
                "account" | "connect" | "disconnect" | "default" | "model" | "session" | "usage"
            )
        {
            subcommand = Some(argument);
        }
    }
    if let (true, Some(subcommand)) = (print_requested, subcommand) {
        Err(raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            format!(
                "-p/--print cannot be combined with the `{subcommand}` subcommand; use `--` before a literal prompt named `{subcommand}`"
            ),
        ))
    } else {
        Ok(())
    }
}

fn raw_command_error(kind: clap::error::ErrorKind, message: impl Into<String>) -> clap::Error {
    let mut message = message.into();
    if !message.ends_with('\n') {
        message.push('\n');
    }
    clap::Error::raw(kind, message)
}

fn glyph_profile(ascii: bool) -> GlyphProfile {
    if ascii {
        GlyphProfile::Ascii
    } else {
        GlyphProfile::Rich
    }
}

#[cfg(test)]
mod tests;
