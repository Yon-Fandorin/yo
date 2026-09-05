use std::{
    env,
    error::Error,
    fmt,
    io::{self, IsTerminal, Write},
};

use crate::interaction::{PresentationStyle, TextStyle};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CliDiagnostic {
    message: String,
}

impl CliDiagnostic {
    pub(crate) fn warning(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug)]
pub(crate) struct AppError {
    failures: Vec<AppFailure>,
    help: Vec<String>,
}

impl AppError {
    pub(crate) fn single(context: &'static str, error: impl fmt::Display) -> Self {
        Self {
            failures: vec![AppFailure::Context {
                context,
                cause: error.to_string(),
            }],
            help: Vec::new(),
        }
    }

    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self::many([message.into()])
    }

    pub(crate) fn many<I, T>(failures: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<AppFailure>,
    {
        Self {
            failures: failures.into_iter().map(Into::into).collect(),
            help: Vec::new(),
        }
    }

    pub(crate) fn combine(errors: impl IntoIterator<Item = Self>) -> Self {
        let mut combined = Self::many(Vec::<AppFailure>::new());
        for error in errors {
            combined.failures.extend(error.failures);
            for command in error.help {
                if !combined.help.contains(&command) {
                    combined.help.push(command);
                }
            }
        }
        combined
    }

    pub(crate) fn with_help<I, T>(mut self, commands: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.help = commands.into_iter().map(Into::into).collect();
        self
    }

    pub(crate) fn print(&self) -> io::Result<()> {
        let stderr = io::stderr();
        let styled =
            PresentationStyle::for_output(stderr.is_terminal(), env::var_os("NO_COLOR").is_some())
                .is_ansi();
        let mut stderr = stderr.lock();
        stderr.write_all(self.render(styled).as_bytes())?;
        stderr.flush()
    }

    fn render(&self, styled: bool) -> String {
        let mut output = String::new();
        if self.failures.len() == 1 {
            render_single(&mut output, &self.failures[0], styled);
        } else {
            push_label(&mut output, "error:", TextStyle::Error, styled);
            output.push_str(" multiple operations failed\n\n");
            for failure in &self.failures {
                push_indented(&mut output, &failure.to_string(), "  - ", "    ");
                output.push('\n');
            }
        }
        if !self.help.is_empty() {
            output.push('\n');
            push_label(&mut output, "tip:", TextStyle::Tip, styled);
            output.push_str(" try one of these commands\n\n");
            for command in &self.help {
                output.push_str("  ");
                output.push_str(command);
                output.push('\n');
            }
        }
        output
    }

    #[cfg(test)]
    pub(crate) fn help(&self) -> &[String] {
        &self.help
    }
}

#[derive(Debug)]
pub(crate) enum AppFailure {
    Context {
        context: &'static str,
        cause: String,
    },
    Message(String),
}

impl From<String> for AppFailure {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl fmt::Display for AppFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Context { context, cause } => write!(formatter, "{context}: {cause}"),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, failure) in self.failures.iter().enumerate() {
            if index != 0 {
                formatter.write_str("; additionally, ")?;
            }
            failure.fmt(formatter)?;
        }
        Ok(())
    }
}

impl Error for AppError {}

fn render_single(output: &mut String, failure: &AppFailure, styled: bool) {
    match failure {
        AppFailure::Message(message) => {
            push_label(output, "error:", TextStyle::Error, styled);
            output.push(' ');
            push_indented(output, message, "", "  ");
            output.push('\n');
        },
        AppFailure::Context { context, cause } => {
            push_label(output, "error:", TextStyle::Error, styled);
            output.push(' ');
            output.push_str(context);
            output.push_str(" failed\n\nCaused by:\n");
            push_indented(output, cause, "  ", "  ");
            output.push('\n');
        },
    }
}

fn push_label(output: &mut String, label: &str, text_style: TextStyle, styled: bool) {
    PresentationStyle::for_output(styled, false).push(output, text_style, label);
}

fn push_indented(output: &mut String, value: &str, first: &str, continuation: &str) {
    let mut lines = value.split('\n').peekable();
    let mut first_line = true;
    while let Some(line) = lines.next() {
        output.push_str(if first_line { first } else { continuation });
        output.push_str(line);
        if lines.peek().is_some() {
            output.push('\n');
        }
        first_line = false;
    }
}

#[cfg(test)]
mod tests;
