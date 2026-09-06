use super::live::{LiveSelection, SandboxMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrintOptions {
    pub(crate) prompt: Option<String>,
    pub(crate) selection: LiveSelection,
    pub(crate) model: Option<String>,
    pub(crate) no_tools: bool,
    pub(crate) sandbox: Option<SandboxMode>,
}

pub(super) fn from_cli(
    prompt: Option<String>,
    resume: Option<yo_core::SessionId>,
    model: Option<String>,
    no_tools: bool,
    sandbox: Option<SandboxMode>,
    output: super::output::OutputOptions,
) -> Result<PrintOptions, clap::Error> {
    super::output::validate(output, "print", false, false)?;
    let selection = resume.map_or(LiveSelection::New, LiveSelection::Resume);
    if selection != LiveSelection::New && model.is_some() {
        return Err(super::raw_command_error(
            clap::error::ErrorKind::ArgumentConflict,
            "--print --resume uses the stored model binding and cannot be combined with --model",
        ));
    }
    Ok(PrintOptions {
        prompt,
        selection,
        model,
        no_tools,
        sandbox,
    })
}
