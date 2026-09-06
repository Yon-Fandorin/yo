use clap::ValueEnum;
use yo_tui::{GlyphProfile, PresentationMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiveOptions {
    pub(crate) mode: PresentationMode,
    pub(crate) glyph_profile: GlyphProfile,
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

pub(super) fn from_cli(
    mode: PresentationMode,
    output: super::output::OutputOptions,
    resume: Option<yo_core::SessionId>,
    continue_session: bool,
    model: Option<String>,
    no_tools: bool,
    sandbox: Option<SandboxMode>,
) -> Result<LiveOptions, clap::Error> {
    super::output::validate(output, "live", false, true)?;
    let selection = match (resume, continue_session) {
        (Some(session_id), false) => LiveSelection::Resume(session_id),
        (None, true) => LiveSelection::Continue,
        (None, false) => LiveSelection::New,
        (Some(_), true) => unreachable!("clap rejects conflicting continuation options"),
    };
    Ok(LiveOptions {
        mode,
        glyph_profile: output.glyph_profile,
        selection,
        model,
        no_tools,
        sandbox,
    })
}
