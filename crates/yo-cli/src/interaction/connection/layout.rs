use std::{fmt::Write as _, mem, num::NonZeroU16};

use unicode_segmentation::UnicodeSegmentation;
use yo_tui::surface::Grapheme;

use super::{
    error::PresentationError,
    plan::{PlanAction, PlanCounts},
};
use crate::interaction::{PresentationStyle, TextStyle};

const DEFAULT_WIDTH: u16 = 80;
pub(super) const FIELD_LABEL_WIDTH: usize = 16;
pub(super) const FIELD_INDENT: usize = 2;

pub(crate) fn default_width() -> NonZeroU16 {
    NonZeroU16::new(DEFAULT_WIDTH).expect("the default terminal width is nonzero")
}

pub(crate) fn push_title(
    output: &mut String,
    title: &str,
    target: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline = format!("{title}  {target}");
    if safe_width(&inline)? <= width {
        style.push(output, TextStyle::Accent, title);
        output.push_str("  ");
        style.push(output, TextStyle::Bold, target);
        output.push('\n');
        return Ok(());
    }
    for line in wrap(title, width)? {
        style.push(output, TextStyle::Accent, &line);
        output.push('\n');
    }
    for line in wrap(target, width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn push_section_heading(
    output: &mut String,
    heading: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    for line in wrap(heading, width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn push_change(
    output: &mut String,
    action: PlanAction,
    label: &str,
    detail: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    const PREFIX_WIDTH: usize = 2;
    if width <= PREFIX_WIDTH {
        style.push(output, action.text_style(), action.marker());
        output.push('\n');
        for line in wrap(label, width)? {
            style.push(output, TextStyle::Bold, &line);
            output.push('\n');
        }
    } else {
        for (index, line) in wrap(label, width - PREFIX_WIDTH)?.iter().enumerate() {
            if index == 0 {
                style.push(output, action.text_style(), action.marker());
                output.push(' ');
            } else {
                output.push_str("  ");
            }
            style.push(output, TextStyle::Bold, line);
            output.push('\n');
        }
    }
    let detail_indent = 2_usize;
    if width <= detail_indent {
        for line in wrap(detail, width)? {
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
    } else {
        for line in wrap(detail, width - detail_indent)? {
            output.push_str("  ");
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
    }
    Ok(())
}

pub(crate) fn push_detail_field(
    output: &mut String,
    label: &str,
    value: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let inline_prefix = FIELD_INDENT + FIELD_LABEL_WIDTH;
    if safe_width(label)? <= FIELD_LABEL_WIDTH && width > inline_prefix {
        let prefix = format!("  {label:<FIELD_LABEL_WIDTH$}");
        for (index, line) in wrap(value, width - inline_prefix)?.iter().enumerate() {
            if index == 0 {
                style.push(output, TextStyle::Muted, &prefix);
            } else {
                style.push(output, TextStyle::Muted, &" ".repeat(inline_prefix));
            }
            style.push(output, TextStyle::Muted, line);
            output.push('\n');
        }
    } else {
        let indent = FIELD_INDENT.min(width.saturating_sub(1));
        let content_width = width.saturating_sub(indent).max(1);
        for line in wrap(label, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
        for line in wrap(value, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
    }
    Ok(())
}

pub(crate) fn push_model_list_field(
    output: &mut String,
    label: &str,
    values: &[&str],
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    let displayed = values
        .iter()
        .map(|value| display_model_item(value))
        .collect::<Vec<_>>();
    let displayed = displayed.iter().map(String::as_str).collect::<Vec<_>>();
    let inline_prefix = FIELD_INDENT + FIELD_LABEL_WIDTH;
    let inline_width = width.saturating_sub(inline_prefix);
    let every_item_fits = displayed.iter().enumerate().all(|(index, value)| {
        safe_width(value).is_ok_and(|item_width| {
            item_width + usize::from(index + 1 < displayed.len()) <= inline_width
        })
    });
    if safe_width(label)? <= FIELD_LABEL_WIDTH && width > inline_prefix && every_item_fits {
        let prefix = format!("  {label:<FIELD_LABEL_WIDTH$}");
        let continuation = " ".repeat(inline_prefix);
        for (index, line) in wrap_list(&displayed, inline_width)?.iter().enumerate() {
            style.push(
                output,
                TextStyle::Muted,
                if index == 0 { &prefix } else { &continuation },
            );
            style.push(output, TextStyle::Muted, line);
            output.push('\n');
        }
    } else {
        let indent = FIELD_INDENT.min(width.saturating_sub(1));
        let content_width = width.saturating_sub(indent).max(1);
        for line in wrap(label, content_width)? {
            output.push_str(&" ".repeat(indent));
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
        for value in displayed {
            push_bullet(output, value, width, style)?;
        }
    }
    Ok(())
}

pub(crate) fn display_model_item(model: &str) -> String {
    if model.chars().any(|character| {
        character == ',' || character == '"' || character == '\\' || character.is_whitespace()
    }) {
        serde_json::to_string(model).expect("serializing a model ID string cannot fail")
    } else {
        model.to_owned()
    }
}

pub(crate) fn escape_remote_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if matches!(byte, 0x20..=0x7e) && !matches!(byte, b'"' | b'\\') {
            escaped.push(char::from(byte));
        } else {
            write!(escaped, "\\x{byte:02X}").expect("formatting into a String cannot fail");
        }
    }
    escaped
}

pub(super) fn wrap_list(values: &[&str], width: usize) -> Result<Vec<String>, PresentationError> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0_usize;
    for (index, value) in values.iter().enumerate() {
        let item = if index + 1 == values.len() {
            (*value).to_owned()
        } else {
            format!("{value},")
        };
        let item_width = safe_width(&item)?;
        let separator_width = usize::from(!line.is_empty());
        if !line.is_empty() && used + separator_width + item_width > width {
            lines.push(mem::take(&mut line));
            used = 0;
        }
        if item_width <= width {
            if !line.is_empty() {
                line.push(' ');
                used += 1;
            }
            line.push_str(&item);
            used += item_width;
            continue;
        }
        if !line.is_empty() {
            lines.push(mem::take(&mut line));
        }
        let mut wrapped = wrap(&item, width)?;
        line = wrapped
            .pop()
            .expect("wrap always returns at least one line");
        used = safe_width(&line)?;
        lines.extend(wrapped);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    Ok(lines)
}

pub(crate) fn push_plan_summary(
    output: &mut String,
    counts: &PlanCounts,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    for line in wrap(&counts.sentence(), width)? {
        style.push(output, TextStyle::Bold, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn push_bullet(
    output: &mut String,
    value: &str,
    width: usize,
    style: PresentationStyle,
) -> Result<(), PresentationError> {
    const PREFIX: &str = "  • ";
    const PREFIX_WIDTH: usize = 4;
    if width <= PREFIX_WIDTH {
        for line in wrap(value, width)? {
            style.push(output, TextStyle::Muted, &line);
            output.push('\n');
        }
        return Ok(());
    }
    for (index, line) in wrap(value, width.saturating_sub(PREFIX_WIDTH).max(1))?
        .into_iter()
        .enumerate()
    {
        style.push(
            output,
            TextStyle::Muted,
            if index == 0 { PREFIX } else { "    " },
        );
        style.push(output, TextStyle::Muted, &line);
        output.push('\n');
    }
    Ok(())
}

pub(crate) fn wrap(value: &str, width: usize) -> Result<Vec<String>, PresentationError> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0_usize;
    for grapheme in value.graphemes(true) {
        let grapheme_width = usize::from(Grapheme::try_from(grapheme)?.width().get());
        if grapheme_width > width {
            return Err(PresentationError::GraphemeExceedsWidth {
                grapheme_width,
                width,
            });
        }
        if !line.is_empty() && used + grapheme_width > width {
            lines.push(mem::take(&mut line));
            used = 0;
        }
        line.push_str(grapheme);
        used += grapheme_width;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    Ok(lines)
}

pub(super) fn safe_width(value: &str) -> Result<usize, PresentationError> {
    value.graphemes(true).try_fold(0_usize, |width, text| {
        let grapheme = Grapheme::try_from(text)?;
        Ok(width + usize::from(grapheme.width().get()))
    })
}

pub(super) fn widest_grapheme(value: &str) -> Result<usize, PresentationError> {
    value.graphemes(true).try_fold(0_usize, |width, text| {
        let grapheme = Grapheme::try_from(text)?;
        Ok(width.max(usize::from(grapheme.width().get())))
    })
}

pub(crate) fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

pub(crate) fn trim_trailing_newline(output: &mut String) {
    while output.ends_with('\n') {
        output.pop();
    }
}
