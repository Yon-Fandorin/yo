use std::{env, io, io::IsTerminal as _, num::NonZeroU16, os::fd};

use rustix::termios;

use super::{
    error::PresentationError,
    layout::{FIELD_INDENT, FIELD_LABEL_WIDTH, default_width, safe_width, widest_grapheme, wrap},
};
use crate::interaction::{PresentationStyle, TextStyle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SuccessPresentation {
    pub(super) width: NonZeroU16,
    pub(super) style: PresentationStyle,
}

impl SuccessPresentation {
    pub(crate) fn for_stdout() -> Self {
        let stdout = io::stdout();
        let terminal = stdout.is_terminal();
        Self::for_output(&stdout, terminal, env::var_os("NO_COLOR").is_some())
    }

    pub(super) fn for_output(output: &impl fd::AsFd, terminal: bool, no_color: bool) -> Self {
        let width = terminal
            .then(|| termios::tcgetwinsize(output).ok())
            .flatten()
            .and_then(|size| NonZeroU16::new(size.ws_col))
            .unwrap_or_else(default_width);
        let style = PresentationStyle::for_output(terminal, no_color);
        Self { width, style }
    }

    #[cfg(test)]
    pub(crate) const fn plain(width: NonZeroU16) -> Self {
        Self {
            width,
            style: PresentationStyle::Plain,
        }
    }

    #[cfg(test)]
    pub(crate) const fn ansi(width: NonZeroU16) -> Self {
        Self {
            width,
            style: PresentationStyle::Ansi,
        }
    }
}

pub(crate) fn render_success(
    presentation: SuccessPresentation,
    heading: &str,
    label_width: usize,
    fields: &[(&str, String)],
) -> Result<String, PresentationError> {
    let width = usize::from(presentation.width.get());
    let style = presentation.style;
    let mut output = String::new();
    push_success_heading(&mut output, heading, width, style)?;
    output.push('\n');
    for (label, value) in fields {
        push_success_field(&mut output, label, value, label_width, width)?;
    }
    Ok(output)
}

fn push_success_heading(
    output: &mut String,
    heading: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline = format!("✓ {heading}");
    if safe_width(&inline)? <= width {
        style.push(output, TextStyle::Positive, "✓");
        output.push(' ');
        style.push(output, TextStyle::Bold, heading);
        output.push('\n');
        return Ok(());
    }
    style.push(output, TextStyle::Positive, "✓");
    output.push('\n');
    for line in wrap(heading, width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

fn push_success_field(
    output: &mut String,
    label: &str,
    value: &str,
    label_width: usize,
    width: usize,
) -> Result<(), PresentationError> {
    let inline_prefix = FIELD_INDENT + label_width;
    let inline_width = width.saturating_sub(inline_prefix);
    if safe_width(label)? <= label_width
        && width > inline_prefix
        && widest_grapheme(value)? <= inline_width
    {
        let prefix = format!("  {label:<label_width$}");
        let continuation = " ".repeat(inline_prefix);
        for (index, line) in wrap(value, inline_width)?.iter().enumerate() {
            output.push_str(if index == 0 { &prefix } else { &continuation });
            output.push_str(line);
            output.push('\n');
        }
    } else {
        let content_width = width;
        for line in wrap(label, content_width)? {
            output.push_str(&line);
            output.push('\n');
        }
        for line in wrap(value, content_width)? {
            output.push_str(&line);
            output.push('\n');
        }
    }
    Ok(())
}
