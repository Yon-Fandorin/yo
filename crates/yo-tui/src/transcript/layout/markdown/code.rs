//! Language parsing and semantic syntax roles; no terminal output.
use std::sync::OnceLock;

use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

use super::{Block, Decoration, Role};

// Cache immutable syntax definitions; retain literal source offsets across wrapped rows.
pub(super) fn highlight_code(block: &mut Block, language: &str) {
    if block.text.len() > 65_536 || block.text.lines().any(|line| line.len() > 4096) {
        return;
    }
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let language = match language {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "sh" | "shell" => "bash",
        other => other,
    };
    let Some(syntax) = syntaxes.find_syntax_by_token(language) else {
        return;
    };
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    let Some(theme) = themes.themes.get("base16-ocean.dark") else {
        return;
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in block.text.split_inclusive('\n') {
        let Ok(tokens) = highlighter.highlight_line(line, syntaxes) else {
            return;
        };
        for (style, text) in tokens {
            let color = style.foreground;
            let role = match (color.r, color.g, color.b) {
                (180, 142, 173) => Role::Syntax(0),
                (163, 190, 140) | (150, 181, 180) => Role::Syntax(1),
                (101, 115, 126) => Role::Syntax(2),
                (208, 135, 112) | (171, 121, 103) => Role::Syntax(3),
                (143, 161, 179) => Role::Syntax(4),
                (235, 203, 139) => Role::Syntax(5),
                _ => Role::Code,
            };
            spans.push((offset, Decoration::role(role)));
            offset += text.len();
        }
    }
    block.spans = spans;
}
