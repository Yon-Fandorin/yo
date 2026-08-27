use std::{env, io::IsTerminal as _};

const ANSI_RESET: &str = "\u{1b}[0m";

/// Whether semantic CLI text may include ANSI decoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PresentationStyle {
    Plain,
    Ansi,
}

impl PresentationStyle {
    pub(crate) fn for_stdout() -> Self {
        Self::for_output(
            std::io::stdout().is_terminal(),
            env::var_os("NO_COLOR").is_some(),
        )
    }

    /// A caller that already owns a controlling terminal may use styling even when stdout
    /// itself is redirected. `NO_COLOR` remains authoritative.
    pub(crate) fn for_controlling_terminal() -> Self {
        Self::for_output(true, env::var_os("NO_COLOR").is_some())
    }

    pub(crate) const fn for_output(terminal: bool, no_color: bool) -> Self {
        if terminal && !no_color {
            Self::Ansi
        } else {
            Self::Plain
        }
    }

    pub(crate) const fn is_ansi(self) -> bool {
        matches!(self, Self::Ansi)
    }

    pub(crate) fn push(self, output: &mut String, text_style: TextStyle, value: &str) {
        if self == Self::Ansi {
            output.push_str(text_style.ansi());
            output.push_str(value);
            output.push_str(ANSI_RESET);
        } else {
            output.push_str(value);
        }
    }

    pub(crate) fn decorate(self, text_style: TextStyle, value: &str) -> String {
        let mut output = String::new();
        self.push(&mut output, text_style, value);
        output
    }
}

/// Semantic roles shared by non-interactive CLI presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextStyle {
    Bold,
    Accent,
    Positive,
    Warning,
    Danger,
    Muted,
    Error,
    Tip,
    Selected,
}

impl TextStyle {
    const fn ansi(self) -> &'static str {
        match self {
            Self::Bold => "\u{1b}[1m",
            Self::Accent => "\u{1b}[1;36m",
            Self::Positive => "\u{1b}[32m",
            Self::Warning => "\u{1b}[33m",
            Self::Danger => "\u{1b}[31m",
            Self::Muted => "\u{1b}[2m",
            Self::Error => "\u{1b}[1;31m",
            Self::Tip => "\u{1b}[1;36m",
            Self::Selected => "\u{1b}[30;46m",
        }
    }
}

/// Renders a bounded left-to-right bar whose filled cells represent remaining capacity.
pub(crate) fn remaining_bar(remaining_percent: u8, width: usize) -> String {
    let remaining_percent = remaining_percent.min(100);
    let filled = usize::from(remaining_percent) * width / 100;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

#[cfg(test)]
pub(crate) fn strip_ansi(value: &str) -> String {
    [
        TextStyle::Bold,
        TextStyle::Accent,
        TextStyle::Positive,
        TextStyle::Warning,
        TextStyle::Danger,
        TextStyle::Muted,
        TextStyle::Error,
        TextStyle::Tip,
        TextStyle::Selected,
    ]
    .into_iter()
    .fold(value.to_owned(), |value, style| {
        value.replace(style.ansi(), "")
    })
    .replace(ANSI_RESET, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ANSI 허용 여부는 모든 CLI 출력이 공유하는 terminal·NO_COLOR 계약에서 한 번만
    // 결정되어, 개별 명령이 서로 다른 색상 정책을 만들지 않습니다.
    #[test]
    fn output_style_requires_a_terminal_and_respects_no_color() {
        assert!(PresentationStyle::for_output(true, false).is_ansi());
        assert!(!PresentationStyle::for_output(false, false).is_ansi());
        assert!(!PresentationStyle::for_output(true, true).is_ansi());
    }

    // 잔여율 막대는 20칸 기준으로 내림하여 과장하지 않고, 범위를 벗어나는 Provider
    // 값이 들어와도 화면 폭을 넘기지 않습니다.
    #[test]
    fn remaining_bar_is_bounded_and_conservative() {
        assert_eq!(remaining_bar(92, 20), "██████████████████░░");
        assert_eq!(remaining_bar(100, 20), "████████████████████");
        assert_eq!(remaining_bar(101, 20), "████████████████████");
    }
}
