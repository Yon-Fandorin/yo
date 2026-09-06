use yo_core::{AccountCapacityBucket, AccountCapacitySnapshot};
use yo_tui::{GlyphProfile, surface::cell_width};

use super::{
    account_table_meter,
    common::{
        account_scope_label, capacity_tone, display_age, display_credits,
        display_observed_at_with_age, display_plan, display_status, is_additional_bucket,
    },
};
use crate::{
    command::account::domain::{
        AccountCapacityRecord, AccountQuery, AccountTarget, display_identifier, terminal_safe,
    },
    presentation::{PresentationStyle, TextStyle},
};

#[cfg(test)]
mod tests;

const TABLE_ACCOUNT_HEADINGS: [&str; 5] = ["PROVIDER", "ACCOUNT", "PLAN", "LIMITS", "UPDATED"];

#[derive(Clone, Debug)]
struct TableCell {
    plain: String,
    parts: Vec<TableCellPart>,
}

#[derive(Clone, Debug)]
struct TableCellPart {
    value: String,
    tone: Option<TextStyle>,
}

impl TableCell {
    fn from_parts(parts: Vec<TableCellPart>) -> Self {
        let mut plain = String::new();
        for part in &parts {
            plain.push_str(&part.value);
        }
        Self { plain, parts }
    }

    fn plain(value: impl Into<String>) -> Self {
        Self::from_parts(vec![TableCellPart {
            value: value.into(),
            tone: None,
        }])
    }

    fn styled(value: impl Into<String>, tone: TextStyle) -> Self {
        Self::from_parts(vec![TableCellPart {
            value: value.into(),
            tone: Some(tone),
        }])
    }

    fn width(&self) -> usize {
        cell_width(&self.plain).expect("account table cells must be terminal-safe")
    }

    fn render(
        &self,
        output: &mut String,
        style: PresentationStyle,
        default_tone: Option<TextStyle>,
    ) {
        for part in &self.parts {
            match part.tone.or(default_tone) {
                Some(tone) => style.push(output, tone, &part.value),
                None => output.push_str(&part.value),
            }
        }
    }
}

fn table_account_row(
    record: &AccountCapacityRecord,
    glyph_profile: GlyphProfile,
) -> Vec<TableCell> {
    let Some(report) = record.report.as_ref() else {
        return match &record.target {
            AccountTarget::LocalHost { provider } => vec![
                TableCell::plain(provider.as_str()),
                TableCell::styled("Local host", TextStyle::Muted),
                TableCell::styled("Unknown", TextStyle::Muted),
                TableCell::styled("Not refreshed", TextStyle::Muted),
                TableCell::styled("Never", TextStyle::Muted),
            ],
            AccountTarget::Exact(coordinate) => vec![
                TableCell::plain(coordinate.provider.as_str()),
                TableCell::plain(terminal_safe(coordinate.account.as_str())),
                TableCell::styled("Unknown", TextStyle::Muted),
                TableCell::styled("Not refreshed", TextStyle::Muted),
                TableCell::styled("Never", TextStyle::Muted),
            ],
        };
    };

    let snapshot = report.snapshot();
    let (status, status_tone) = display_status(snapshot);
    let mut limits = table_limit_cell(snapshot, glyph_profile);
    if status != "Available" {
        let mut parts = vec![TableCellPart {
            value: status,
            tone: Some(status_tone),
        }];
        if !limits.plain.is_empty() {
            parts.push(TableCellPart {
                value: " · ".to_owned(),
                tone: None,
            });
        }
        parts.extend(limits.parts);
        limits = TableCell::from_parts(parts);
    }

    let updated = report.observed_at().map_or_else(
        || TableCell::styled("Never", TextStyle::Muted),
        table_observed_at_cell,
    );
    vec![
        TableCell::plain(snapshot.provider().as_str()),
        TableCell::plain(terminal_safe(report.account_label())),
        TableCell::plain(display_plan(snapshot)),
        limits,
        updated,
    ]
}

pub(super) fn render_table_records(
    records: &[AccountCapacityRecord],
    query: &AccountQuery,
    style: PresentationStyle,
    glyph_profile: GlyphProfile,
) -> String {
    let mut output = String::new();
    style.push(
        &mut output,
        TextStyle::Bold,
        &format!(
            "{} · {} {}",
            account_scope_label(query),
            records.len(),
            if records.len() == 1 {
                "account"
            } else {
                "accounts"
            }
        ),
    );
    output.push('\n');

    let headings = TABLE_ACCOUNT_HEADINGS
        .iter()
        .map(|heading| TableCell::plain(*heading))
        .collect::<Vec<_>>();
    let rows = records
        .iter()
        .map(|record| table_account_row(record, glyph_profile))
        .collect::<Vec<_>>();
    let widths = table_column_widths(&headings, &rows);
    push_table_row(
        &mut output,
        &headings,
        &widths,
        style,
        Some(TextStyle::Muted),
    );
    for row in &rows {
        push_table_row(&mut output, row, &widths, style, None);
    }
    output
}

fn table_column_widths(headings: &[TableCell], rows: &[Vec<TableCell>]) -> Vec<usize> {
    let mut widths = headings.iter().map(TableCell::width).collect::<Vec<_>>();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.width());
        }
    }
    widths
}

fn push_table_row(
    output: &mut String,
    cells: &[TableCell],
    widths: &[usize],
    style: PresentationStyle,
    default_tone: Option<TextStyle>,
) {
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        cell.render(output, style, default_tone);
        if index + 1 < cells.len() {
            output.push_str(&" ".repeat(width.saturating_sub(cell.width()) + 2));
        }
    }
    output.push('\n');
}

fn table_limit_cell(snapshot: &AccountCapacitySnapshot, glyph_profile: GlyphProfile) -> TableCell {
    let mut parts = Vec::new();
    for (index, (value, tone)) in table_limit_items(snapshot, glyph_profile)
        .into_iter()
        .enumerate()
    {
        if index != 0 {
            parts.push(TableCellPart {
                value: " · ".to_owned(),
                tone: None,
            });
        }
        parts.push(TableCellPart {
            value,
            tone: Some(tone),
        });
    }
    TableCell::from_parts(parts)
}

fn table_limit_items(
    snapshot: &AccountCapacitySnapshot,
    glyph_profile: GlyphProfile,
) -> Vec<(String, TextStyle)> {
    let mut limits = Vec::new();
    for (index, bucket) in snapshot.buckets().iter().enumerate() {
        let nested = is_additional_bucket(snapshot, bucket, index);
        let bucket_label = nested.then(|| table_bucket_label(bucket));
        for window in [bucket.primary(), bucket.secondary()].into_iter().flatten() {
            let mut label = table_window_label(window.window_duration_minutes());
            if let Some(bucket_label) = &bucket_label {
                label = format!("{bucket_label} {label}");
            }
            let remaining = window.remaining_percent_basis_points();
            let rendered = account_table_meter(glyph_profile)
                .render(&label, remaining)
                .expect("built-in account limit meter must remain renderable");
            limits.push((rendered, capacity_tone(remaining)));
        }
    }
    if limits.is_empty()
        && let Some(credits) = snapshot
            .buckets()
            .iter()
            .find_map(|bucket| bucket.credits())
    {
        return vec![(
            format!("credits {}", display_credits(credits)),
            TextStyle::Muted,
        )];
    }
    if limits.is_empty() {
        return vec![("No capacity".to_owned(), TextStyle::Muted)];
    }
    limits
}

fn table_bucket_label(bucket: &AccountCapacityBucket) -> String {
    let value = bucket
        .name()
        .or_else(|| bucket.id())
        .unwrap_or("Additional");
    if value.to_ascii_lowercase().contains("spark") {
        return "Spark".to_owned();
    }
    let value = display_identifier(value);
    let mut characters = value.chars();
    let compact = characters.by_ref().take(15).collect::<String>();
    if characters.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

fn table_window_label(minutes: Option<u64>) -> String {
    let Some(minutes) = minutes else {
        return "window".to_owned();
    };
    const MINUTES_PER_DAY: u64 = 24 * 60;
    if minutes == 7 * MINUTES_PER_DAY {
        return "7d".to_owned();
    }
    if minutes != 0 && minutes.is_multiple_of(60) {
        return format!("{}h", minutes / 60);
    }
    format!("{minutes}m")
}

fn table_observed_at_cell(value: &str) -> TableCell {
    let Ok(timestamp) = value.parse::<jiff::Timestamp>() else {
        return TableCell::plain(display_observed_at_with_age(value));
    };
    let displayed = timestamp
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M %Z")
        .to_string();
    let age = jiff::Timestamp::now()
        .as_second()
        .saturating_sub(timestamp.as_second());
    if age < 0 {
        return TableCell::plain(displayed);
    }
    TableCell::from_parts(vec![
        TableCellPart {
            value: displayed,
            tone: None,
        },
        TableCellPart {
            value: format!(" ({})", display_age(age as u64)),
            tone: Some(TextStyle::Muted),
        },
    ])
}
