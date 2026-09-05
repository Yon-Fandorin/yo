use std::fmt::Write as _;

use unicode_segmentation::UnicodeSegmentation;
use yo_core::{AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow};
use yo_tui::{GlyphProfile, surface::cell_width};

use super::{
    AccountOutputWidth,
    common::{
        additional_bucket_heading, capacity_tone, display_credits, display_observed_at_with_age,
        display_plan, display_status, is_additional_bucket,
    },
};
use crate::{
    command::account::domain::{
        AccountCapacityRecord, AccountCapacityReport, AccountTarget, display_identifier,
        local_host_label, terminal_safe,
    },
    interaction::{PresentationStyle, TextStyle, remaining_bar},
};

#[cfg(test)]
mod tests;

pub(super) fn render_detailed_records(
    records: &[AccountCapacityRecord],
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) -> String {
    let mut output = String::new();
    if records.len() > 1 {
        let heading = format!("Account capacity · {} accounts", records.len());
        if let Some(width) = width {
            push_wrapped_value(&mut output, "", &heading, width, style, TextStyle::Bold);
        } else {
            style.push(&mut output, TextStyle::Bold, &heading);
            output.push('\n');
        }
        output.push('\n');
    }
    for (index, record) in records.iter().enumerate() {
        if index != 0 {
            output.push('\n');
        }
        match &record.report {
            Some(report) => output.push_str(&render_report(report, style, glyph_profile, width)),
            None => output.push_str(&render_missing(&record.target, style, width)),
        }
    }
    output
}

pub(super) fn detail_width(output_width: AccountOutputWidth) -> Option<usize> {
    match output_width {
        AccountOutputWidth::Bounded(width) => Some(usize::from(width.get())),
        AccountOutputWidth::Unbounded | AccountOutputWidth::Unknown => None,
    }
}

fn render_missing(
    target: &AccountTarget,
    style: PresentationStyle,
    width: Option<usize>,
) -> String {
    let mut output = String::new();
    let (heading, account, heading_account) = match target {
        AccountTarget::LocalHost { provider } => {
            (local_host_label(provider), "Not resolved".to_owned(), None)
        },
        AccountTarget::Exact(coordinate) => (
            display_identifier(coordinate.provider.as_str()),
            terminal_safe(coordinate.account.as_str()),
            Some(terminal_safe(coordinate.account.as_str())),
        ),
    };
    push_account_heading(
        &mut output,
        &heading,
        heading_account.as_deref(),
        style,
        width,
    );
    if heading_account.is_none() {
        push_field(&mut output, "Account", &account, style, None, width);
    }
    push_field(&mut output, "Plan", "Unknown", style, None, width);
    push_field(
        &mut output,
        "Status",
        "Not refreshed",
        style,
        Some(TextStyle::Muted),
        width,
    );
    push_field(
        &mut output,
        "Updated",
        "Never",
        style,
        Some(TextStyle::Muted),
        width,
    );
    output.push('\n');
    push_heading(&mut output, "Usage limits", style, width, TextStyle::Accent);
    if let Some(width) = width {
        push_wrapped_value(
            &mut output,
            "  ",
            "No cached capacity information.",
            width,
            style,
            TextStyle::Muted,
        );
    } else {
        style.push(
            &mut output,
            TextStyle::Muted,
            "  No cached capacity information.",
        );
        output.push('\n');
    }
    output
}

#[cfg(test)]
fn render(snapshot: &AccountCapacitySnapshot, style: PresentationStyle) -> String {
    render_snapshot(snapshot, None, style, GlyphProfile::Rich, None)
}

fn render_report(
    report: &AccountCapacityReport,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) -> String {
    render_snapshot(
        report.snapshot(),
        report.observed_at(),
        style,
        glyph_profile,
        width,
    )
}

fn push_account_heading(
    output: &mut String,
    provider: &str,
    account: Option<&str>,
    style: PresentationStyle,
    width: Option<usize>,
) {
    if let Some(width) = width {
        let provider_overflows = line_width(provider) > width;
        let heading_overflows =
            account.is_some_and(|account| line_width(&format!("{provider} · {account}")) > width);
        if provider_overflows || heading_overflows {
            push_wrapped_value(output, "", provider, width, style, TextStyle::Bold);
            if let Some(account) = account {
                push_wrapped_value(output, "  ", account, width, style, TextStyle::Accent);
            }
            return;
        }
    }
    style.push(output, TextStyle::Bold, provider);
    if let Some(account) = account {
        output.push_str(" · ");
        style.push(output, TextStyle::Accent, account);
    }
    output.push('\n');
}

fn render_snapshot(
    snapshot: &AccountCapacitySnapshot,
    observed_at: Option<&str>,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) -> String {
    let mut output = String::new();
    let provider = display_identifier(snapshot.provider().as_str());
    let account = terminal_safe(snapshot.account_label());
    push_account_heading(&mut output, &provider, Some(&account), style, width);
    push_field(
        &mut output,
        "Plan",
        &display_plan(snapshot),
        style,
        None,
        width,
    );
    let (status, status_tone) = display_status(snapshot);
    push_field(
        &mut output,
        "Status",
        &status,
        style,
        Some(status_tone),
        width,
    );
    if let Some(credits) = snapshot
        .buckets()
        .iter()
        .find_map(|bucket| bucket.credits())
    {
        push_field(
            &mut output,
            "Credits",
            &display_credits(credits),
            style,
            None,
            width,
        );
    }
    if let Some(observed_at) = observed_at {
        push_field(
            &mut output,
            "Updated",
            &display_observed_at_with_age(observed_at),
            style,
            None,
            width,
        );
    }

    output.push('\n');
    push_heading(&mut output, "Usage limits", style, width, TextStyle::Accent);
    if !snapshot.buckets().iter().any(|bucket| {
        bucket.primary().is_some() || bucket.secondary().is_some() || bucket.credits().is_some()
    }) {
        if let Some(width) = width {
            push_wrapped_value(
                &mut output,
                "  ",
                "No capacity information available.",
                width,
                style,
                TextStyle::Muted,
            );
        } else {
            style.push(
                &mut output,
                TextStyle::Muted,
                "  No capacity information available.",
            );
            output.push('\n');
        }
        return output;
    }

    for (index, bucket) in snapshot.buckets().iter().enumerate() {
        let nested = is_additional_bucket(snapshot, bucket, index);
        if nested {
            let heading = format!("{} limit:", additional_bucket_heading(bucket));
            if let Some(width) = width {
                push_wrapped_value(&mut output, "  ", &heading, width, style, TextStyle::Bold);
            } else {
                output.push_str("  ");
                style.push(&mut output, TextStyle::Bold, &heading);
                output.push('\n');
            }
        }
        render_bucket(&mut output, bucket, nested, style, glyph_profile, width);
    }
    output
}

fn push_field(
    output: &mut String,
    label: &str,
    value: &str,
    style: PresentationStyle,
    tone: Option<TextStyle>,
    width: Option<usize>,
) {
    let Some(width) = width else {
        output.push_str("  ");
        style.push(output, TextStyle::Muted, &format!("{label:<9}"));
        match tone {
            Some(tone) => style.push(output, tone, value),
            None => output.push_str(value),
        }
        output.push('\n');
        return;
    };

    let field_indent = "  ";
    let inline_indent_width = cell_width("  ").expect("account field indent must be terminal-safe");
    let inline_prefix_width = inline_indent_width + 9;
    if width > inline_prefix_width {
        let value_width = width - inline_prefix_width;
        let lines = wrap_terminal_text(value, value_width);
        for (index, line) in lines.iter().enumerate() {
            if index == 0 {
                output.push_str(field_indent);
                style.push(output, TextStyle::Muted, &format!("{label:<9}"));
            } else {
                output.push_str(&" ".repeat(inline_prefix_width));
            }
            push_styled_value(output, line, style, tone);
            output.push('\n');
        }
        return;
    }

    let label_indent_width = inline_indent_width.min(width.saturating_sub(1));
    let label_indent = " ".repeat(label_indent_width);
    let label_width = width.saturating_sub(label_indent_width).max(1);
    for line in wrap_terminal_text(label, label_width) {
        output.push_str(&label_indent);
        style.push(output, TextStyle::Muted, &line);
        output.push('\n');
    }
    let value_indent_width = inline_indent_width.min(width.saturating_sub(1));
    let value_indent = " ".repeat(value_indent_width);
    let value_width = width.saturating_sub(value_indent_width).max(1);
    for line in wrap_terminal_text(value, value_width) {
        output.push_str(&value_indent);
        push_styled_value(output, &line, style, tone);
        output.push('\n');
    }
}

fn push_styled_value(
    output: &mut String,
    value: &str,
    style: PresentationStyle,
    tone: Option<TextStyle>,
) {
    match tone {
        Some(tone) => style.push(output, tone, value),
        None => output.push_str(value),
    }
}

pub(super) fn push_heading(
    output: &mut String,
    value: &str,
    style: PresentationStyle,
    width: Option<usize>,
    tone: TextStyle,
) {
    if let Some(width) = width {
        push_wrapped_value(output, "", value, width, style, tone);
    } else {
        style.push(output, tone, value);
        output.push('\n');
    }
}

pub(super) fn push_tail_plain_line(output: &mut String, value: &str, width: Option<usize>) {
    if let Some(width) = width {
        push_wrapped_plain(output, "", value, width);
    } else {
        output.push_str(value);
        output.push('\n');
    }
}

pub(super) fn push_tail_styled_line(
    output: &mut String,
    value: &str,
    style: PresentationStyle,
    tone: TextStyle,
    width: Option<usize>,
) {
    if let Some(width) = width {
        push_wrapped_value(output, "", value, width, style, tone);
    } else {
        style.push(output, tone, value);
        output.push('\n');
    }
}

fn push_wrapped_value(
    output: &mut String,
    indent: &str,
    value: &str,
    width: usize,
    style: PresentationStyle,
    tone: TextStyle,
) {
    let indent_width = cell_width(indent)
        .expect("account value indent must be terminal-safe")
        .min(width.saturating_sub(1));
    let indent = " ".repeat(indent_width);
    let value_width = width.saturating_sub(indent_width).max(1);
    for line in wrap_terminal_text(value, value_width) {
        output.push_str(&indent);
        style.push(output, tone, &line);
        output.push('\n');
    }
}

fn wrap_terminal_text(value: &str, width: usize) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = cell_width(grapheme).unwrap_or(1);
        if grapheme_width > width {
            if !line.is_empty() {
                lines.push(line);
                line = String::new();
                line_width = 0;
            }
            lines.push("?".to_owned());
            continue;
        }
        if !line.is_empty() && line_width + grapheme_width > width {
            lines.push(line);
            line = String::new();
            line_width = 0;
        }
        line.push_str(grapheme);
        line_width += grapheme_width;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn render_bucket(
    output: &mut String,
    bucket: &AccountCapacityBucket,
    nested: bool,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) {
    let windows = [
        ("Primary", bucket.primary().copied()),
        ("Secondary", bucket.secondary().copied()),
    ]
    .into_iter()
    .filter_map(|(label, window)| window.map(|window| (label, window)))
    .collect::<Vec<_>>();
    let (branch_prefix, last_branch_prefix) = match glyph_profile {
        GlyphProfile::Rich => ("  ├─ ", "  └─ "),
        GlyphProfile::Ascii => ("  |-- ", "  `-- "),
        _ => ("  |-- ", "  `-- "),
    };

    for (index, (label, window)) in windows.iter().enumerate() {
        let prefix = if nested {
            if index + 1 == windows.len() {
                last_branch_prefix
            } else {
                branch_prefix
            }
        } else {
            "  "
        };
        render_window(output, prefix, label, *window, style, glyph_profile, width);
    }
}

fn render_window(
    output: &mut String,
    prefix: &str,
    fallback_label: &str,
    window: AccountCapacityWindow,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: Option<usize>,
) {
    let label = window
        .window_duration_minutes()
        .map_or_else(|| format!("{fallback_label} limit:"), display_window_label);
    let remaining = window.remaining_percent_basis_points();
    let tone = capacity_tone(remaining);
    let reset = window
        .resets_at_unix_seconds()
        .map(|reset| format!("(resets {})", display_reset(reset)));
    let full_bar = remaining_bar(window.remaining_percent(), 20, glyph_profile);
    let percent = format!("{}% left", display_percent(remaining));
    let plain_line = format!(
        "{prefix}{label:<15} [{full_bar}] {percent}{}",
        reset
            .as_deref()
            .map_or(String::new(), |value| format!(" {value}"))
    );
    if width.is_none() || width.is_some_and(|width| line_width(&plain_line) <= width) {
        let _ = write!(output, "{prefix}{label:<15} [");
        style.push(output, tone, &full_bar);
        let _ = write!(output, "] {percent}");
        if let Some(reset) = reset {
            let _ = write!(output, " {reset}");
        }
        output.push('\n');
        return;
    }

    render_narrow_window(
        output,
        NarrowWindowOptions {
            prefix,
            label: &label,
            percent: &percent,
            reset: reset.as_deref(),
            remaining_percent: window.remaining_percent(),
            tone,
            style,
            glyph_profile,
            width: width.expect("narrow window rendering requires a bounded width"),
        },
    );
}

struct NarrowWindowOptions<'a> {
    prefix: &'a str,
    label: &'a str,
    percent: &'a str,
    reset: Option<&'a str>,
    remaining_percent: u8,
    tone: TextStyle,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
    width: usize,
}

fn render_narrow_window(output: &mut String, options: NarrowWindowOptions<'_>) {
    let NarrowWindowOptions {
        prefix,
        label,
        percent,
        reset,
        remaining_percent,
        tone,
        style,
        glyph_profile,
        width,
    } = options;
    let label_prefix = format!("{prefix}{label} [");
    let suffix = format!("] {percent}");
    let bar_width = width
        .saturating_sub(line_width(&label_prefix) + line_width(&suffix))
        .min(20);
    if bar_width > 0 {
        output.push_str(&label_prefix);
        style_meter(
            output,
            style,
            tone,
            remaining_percent,
            bar_width,
            glyph_profile,
        );
        output.push_str(&suffix);
        output.push('\n');
    } else {
        push_wrapped_plain(output, "", &format!("{prefix}{label}"), width);
        let indent_width = line_width(prefix).min(width.saturating_sub(1));
        let indent = " ".repeat(indent_width);
        let value_suffix = format!("] {percent}");
        let bar_width = width
            .saturating_sub(indent_width + 1 + line_width(&value_suffix))
            .min(20);
        if bar_width > 0 {
            output.push_str(&indent);
            output.push('[');
            style_meter(
                output,
                style,
                tone,
                remaining_percent,
                bar_width,
                glyph_profile,
            );
            output.push_str(&value_suffix);
            output.push('\n');
        } else {
            output.push_str(&indent);
            style_meter(output, style, tone, remaining_percent, 1, glyph_profile);
            output.push('\n');
            push_wrapped_plain(output, &indent, percent, width);
        }
    }

    if let Some(reset) = reset {
        let indent_width = line_width(prefix).min(width.saturating_sub(1));
        push_wrapped_plain(
            output,
            &" ".repeat(indent_width),
            &format!("resets {reset}"),
            width,
        );
    }
}

fn style_meter(
    output: &mut String,
    style: PresentationStyle,
    tone: TextStyle,
    remaining_percent: u8,
    width: usize,
    glyph_profile: GlyphProfile,
) {
    let meter = remaining_bar(remaining_percent, width, glyph_profile);
    style.push(output, tone, &meter);
}

fn push_wrapped_plain(output: &mut String, indent: &str, value: &str, width: usize) {
    let indent_width = line_width(indent).min(width.saturating_sub(1));
    let indent = " ".repeat(indent_width);
    let value_width = width.saturating_sub(indent_width).max(1);
    for line in wrap_terminal_text(value, value_width) {
        output.push_str(&indent);
        output.push_str(&line);
        output.push('\n');
    }
}

fn line_width(value: &str) -> usize {
    cell_width(value).unwrap_or_else(|_| value.chars().count())
}

fn display_percent(percent_basis_points: u16) -> String {
    yo_tui::meter::format_percent(percent_basis_points)
}

fn display_window_label(minutes: u64) -> String {
    const MINUTES_PER_DAY: u64 = 24 * 60;
    if minutes == 7 * MINUTES_PER_DAY {
        return "Weekly limit:".to_owned();
    }
    if minutes != 0 && minutes.is_multiple_of(60) {
        let hours = minutes / 60;
        return format!("{hours}h limit:");
    }
    format!("{minutes}m limit:")
}

fn display_reset(unix_seconds: i64) -> String {
    jiff::Timestamp::from_second(unix_seconds).map_or_else(
        |_| format!("unix:{unix_seconds}"),
        |timestamp| {
            timestamp
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%b %-d, %H:%M %Z")
                .to_string()
        },
    )
}
