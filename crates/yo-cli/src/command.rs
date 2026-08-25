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
            "resume",
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
    #[arg(long)]
    ascii: bool,

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
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
enum CliCommand {
    /// Connect one service target.
    Connect(ConnectArguments),

    /// Remove one stored external model connection.
    Disconnect(DisconnectArguments),

    /// Show or change the stored startup default.
    Default(DefaultArguments),

    /// List stored Sessions or inspect one Session.
    Session(SessionArguments),

    /// Show usage for one stored Session.
    Usage(UsageArguments),
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

    /// Use ASCII characters instead of rich terminal glyphs.
    #[arg(long, requires = "session_id")]
    ascii: bool,

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

    /// Use ASCII characters instead of rich terminal glyphs.
    #[arg(long)]
    ascii: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiveOptions {
    pub(crate) mode: PresentationMode,
    pub(crate) glyph_profile: GlyphProfile,
    pub(crate) selection: LiveSelection,
    pub(crate) model: Option<String>,
    pub(crate) no_tools: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrintOptions {
    pub(crate) prompt: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) no_tools: bool,
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
    Connect(ConnectCommand),
    Disconnect(DisconnectCommand),
    Default(DefaultCommand),
    Live(LiveOptions),
    Print(PrintOptions),
    Session(SessionCommand),
    Usage(UsageCommand),
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
    pub(crate) glyph_profile: GlyphProfile,
    pub(crate) limit: Option<NonZeroUsize>,
    pub(crate) content: Option<SessionContent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UsageCommand {
    pub(crate) session_id: yo_core::SessionId,
    pub(crate) glyph_profile: GlyphProfile,
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, clap::Error> {
    let cli = Cli::try_parse_from(std::iter::once(OsString::from("yo")).chain(arguments))?;

    match cli.command {
        Some(CliCommand::Connect(arguments)) => Ok(Command::Connect(ConnectCommand {
            target: arguments.target.unwrap_or_default(),
            from: arguments.from,
            verbose: arguments.verbose,
            credential_file: arguments.credential_file,
            yes: arguments.yes,
        })),
        Some(CliCommand::Disconnect(arguments)) => Ok(Command::Disconnect(DisconnectCommand {
            provider: arguments.provider,
            account: arguments.account,
            yes: arguments.yes,
            verbose: arguments.verbose,
        })),
        Some(CliCommand::Default(arguments)) => Ok(Command::Default(DefaultCommand {
            target: arguments.target,
        })),
        Some(CliCommand::Session(arguments)) => {
            let view = arguments.view.unwrap_or(SessionView::Chat);
            if view != SessionView::Transcript
                && (arguments.limit.is_some() || arguments.content.is_some())
            {
                return Err(clap::Error::raw(
                    clap::error::ErrorKind::ArgumentConflict,
                    "--limit and --content are supported only with --view transcript",
                ));
            }
            Ok(Command::Session(SessionCommand {
                session_id: arguments.session_id,
                all: arguments.all,
                details: arguments.details,
                view,
                glyph_profile: glyph_profile(arguments.ascii),
                limit: arguments.limit,
                content: arguments.content,
            }))
        },
        Some(CliCommand::Usage(arguments)) => Ok(Command::Usage(UsageCommand {
            session_id: arguments.session_id,
            glyph_profile: glyph_profile(arguments.ascii),
        })),
        None if cli.print => Ok(Command::Print(PrintOptions {
            prompt: cli.prompt,
            model: cli.model,
            no_tools: cli.no_tools,
        })),
        None => Ok(Command::Live(LiveOptions {
            mode: match (cli.inline, cli.fullscreen) {
                (false, true) => PresentationMode::Fullscreen,
                (_, false) => PresentationMode::Inline,
                (true, true) => unreachable!("clap rejects conflicting presentation modes"),
            },
            glyph_profile: glyph_profile(cli.ascii),
            selection: match (cli.resume, cli.continue_session) {
                (Some(session_id), false) => LiveSelection::Resume(session_id),
                (None, true) => LiveSelection::Continue,
                (None, false) => LiveSelection::New,
                (Some(_), true) => unreachable!("clap rejects conflicting continuation options"),
            },
            model: cli.model,
            no_tools: cli.no_tools,
        })),
    }
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
