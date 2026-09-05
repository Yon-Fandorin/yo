use clap::Args;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub(super) struct Arguments {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) provider: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) yes: bool,
    pub(crate) verbose: bool,
}

impl Arguments {
    pub(super) fn into_command(
        self,
        output: super::output::OutputOptions,
    ) -> Result<Command, clap::Error> {
        super::output::validate(output, "disconnect", false, false)?;
        Ok(Command {
            provider: self.provider,
            account: self.account,
            yes: self.yes,
            verbose: self.verbose,
        })
    }
}
