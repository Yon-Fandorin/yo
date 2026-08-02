use std::ffi::OsString;

use yo_tui::{GlyphProfile, PresentationMode};

use super::AppError;

const USAGE: &str = "yo [--inline | --fullscreen] [--ascii]\n       yo session [--all] [--details]\n       yo session SESSION_ID [--view chat|transcript] [--ascii]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveOptions {
    pub(crate) mode: PresentationMode,
    pub(crate) glyph_profile: GlyphProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Live(LiveOptions),
    Session(SessionCommand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionView {
    Chat,
    Transcript,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionCommand {
    pub(crate) session_id: Option<yo_core::SessionId>,
    pub(crate) all: bool,
    pub(crate) details: bool,
    pub(crate) view: SessionView,
    pub(crate) glyph_profile: GlyphProfile,
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, AppError> {
    let mut arguments = arguments.into_iter().peekable();
    if arguments
        .peek()
        .is_some_and(|argument| argument.as_os_str() == "session")
    {
        arguments.next();
        parse_session(arguments).map(Command::Session)
    } else {
        parse_live(arguments).map(Command::Live)
    }
}

fn parse_live(arguments: impl IntoIterator<Item = OsString>) -> Result<LiveOptions, AppError> {
    let mut mode = None;
    let mut glyph_profile = None;
    for argument in arguments {
        let selected_mode = match argument.as_os_str() {
            value if value == "--inline" => Some(PresentationMode::Inline),
            value if value == "--fullscreen" => Some(PresentationMode::Fullscreen),
            value if value == "--ascii" => {
                set_once(&mut glyph_profile, GlyphProfile::Ascii, "--ascii")?;
                None
            },
            _ => {
                return Err(usage_error(format!(
                    "unknown argument `{}`",
                    argument.to_string_lossy()
                )));
            },
        };
        if let Some(selected_mode) = selected_mode
            && mode.replace(selected_mode).is_some()
        {
            return Err(usage_error("multiple presentation modes"));
        }
    }
    Ok(LiveOptions {
        mode: mode.unwrap_or(PresentationMode::Inline),
        glyph_profile: glyph_profile.unwrap_or(GlyphProfile::Rich),
    })
}

fn parse_session(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<SessionCommand, AppError> {
    let mut session_id = None;
    let mut all = false;
    let mut details = false;
    let mut view = None;
    let mut glyph_profile = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_os_str() {
            value if value == "--all" => set_flag(&mut all, "--all")?,
            value if value == "--details" => set_flag(&mut details, "--details")?,
            value if value == "--ascii" => {
                set_once(&mut glyph_profile, GlyphProfile::Ascii, "--ascii")?;
            },
            value if value == "--view" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| usage_error("`--view` requires `chat` or `transcript`"))?;
                let selected = match value.to_str() {
                    Some("chat") => SessionView::Chat,
                    Some("transcript") => SessionView::Transcript,
                    _ => return Err(usage_error("`--view` requires `chat` or `transcript`")),
                };
                set_once(&mut view, selected, "--view")?;
            },
            value if value.to_string_lossy().starts_with('-') => {
                return Err(usage_error(format!(
                    "unknown argument `{}`",
                    argument.to_string_lossy()
                )));
            },
            _ => {
                let value = argument
                    .to_str()
                    .ok_or_else(|| usage_error("Session ID is not UTF-8"))?;
                let parsed = value
                    .parse()
                    .map_err(|_| usage_error(format!("invalid Session ID `{value}`")))?;
                set_once(&mut session_id, parsed, "SESSION_ID")?;
            },
        }
    }
    if session_id.is_none() && (view.is_some() || glyph_profile.is_some()) {
        return Err(usage_error("`--view` and `--ascii` require a Session ID"));
    }
    if session_id.is_some() && (all || details) {
        return Err(usage_error(
            "`--all` and `--details` apply only to Session lists",
        ));
    }
    Ok(SessionCommand {
        session_id,
        all,
        details,
        view: view.unwrap_or(SessionView::Chat),
        glyph_profile: glyph_profile.unwrap_or(GlyphProfile::Rich),
    })
}

fn set_flag(target: &mut bool, name: &'static str) -> Result<(), AppError> {
    if std::mem::replace(target, true) {
        Err(usage_error(format!("duplicate argument `{name}`")))
    } else {
        Ok(())
    }
}

fn set_once<T>(target: &mut Option<T>, value: T, name: &'static str) -> Result<(), AppError> {
    if target.replace(value).is_some() {
        Err(usage_error(format!("duplicate argument `{name}`")))
    } else {
        Ok(())
    }
}

fn usage_error(message: impl Into<String>) -> AppError {
    AppError::many([format!("{}; usage: {USAGE}", message.into())])
}

#[cfg(test)]
mod tests;
