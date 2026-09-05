use std::{io::IsTerminal as _, num::NonZeroU16};

use yo_tui::{
    GlyphProfile,
    meter::{MeterGlyphs, MeterShape, MeterSpec, MeterTemplate},
    surface::cell_width,
};

use crate::{
    command::account::domain::{
        AccountCapacityRecord, AccountQuery, AccountRefreshFailure, terminal_safe,
    },
    diagnostic::CliDiagnostic,
    presentation::{PresentationStyle, TextStyle},
};

pub(super) mod common;
pub(super) mod detail;
pub(super) mod json;
pub(super) mod table;
const ACCOUNT_TABLE_METER_TEMPLATE: MeterTemplate<'static> =
    MeterTemplate::new("{label} {meter} {percent}%");

pub(super) fn account_table_meter(glyph_profile: GlyphProfile) -> MeterSpec<'static> {
    MeterSpec::new(
        MeterShape::VerticalLevel,
        MeterGlyphs::for_profile(glyph_profile),
        ACCOUNT_TABLE_METER_TEMPLATE,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AccountOutputWidth {
    Unbounded,
    Bounded(NonZeroU16),
    Unknown,
}

pub(super) fn account_output_width() -> AccountOutputWidth {
    if !std::io::stdout().is_terminal() {
        return AccountOutputWidth::Unbounded;
    }
    yo_tui::terminal::current_width()
        .map_or(AccountOutputWidth::Unknown, AccountOutputWidth::Bounded)
}

pub(super) struct AccountRenderOptions<'a> {
    pub(super) query: &'a AccountQuery,
    pub(super) refreshed: bool,
    pub(super) detail: bool,
    pub(super) glyph_profile: GlyphProfile,
    pub(super) warnings: &'a [CliDiagnostic],
    pub(super) failures: &'a [AccountRefreshFailure],
    pub(super) output_width: AccountOutputWidth,
    pub(super) style: PresentationStyle,
}

pub(super) fn render_records_with_width(
    records: &[AccountCapacityRecord],
    options: AccountRenderOptions<'_>,
) -> String {
    let detailed = options.detail || records.len() == 1 || !table_fits(records, &options);
    let AccountRenderOptions {
        query,
        refreshed,
        detail: _,
        glyph_profile,
        warnings,
        failures,
        output_width,
        style,
    } = options;
    let mut output = if detailed {
        detail::render_detailed_records(
            records,
            style,
            glyph_profile,
            detail::detail_width(output_width),
        )
    } else {
        table::render_table_records(records, query, style, glyph_profile)
    };
    let mut commands = Vec::new();
    if !detailed {
        commands.push(("Detail", common::account_command(query, "--detail")));
    }
    if !refreshed && records.iter().any(|record| record.report.is_none()) {
        commands.push(("Refresh", common::account_command(query, "--refresh")));
    }
    if !failures.is_empty() {
        commands.push(("Retry", common::account_command(query, "--refresh")));
    }
    if !warnings.is_empty() {
        output.push('\n');
        detail::push_heading(
            &mut output,
            "Refresh warnings",
            style,
            detail::detail_width(output_width),
            TextStyle::Warning,
        );
        for warning in warnings {
            detail::push_tail_plain_line(
                &mut output,
                &format!("  {}", terminal_safe(warning.message())),
                detail::detail_width(output_width),
            );
        }
    }
    if !failures.is_empty() {
        output.push('\n');
        detail::push_heading(
            &mut output,
            "Refresh failures",
            style,
            detail::detail_width(output_width),
            TextStyle::Error,
        );
        for failure in failures {
            detail::push_tail_plain_line(
                &mut output,
                &format!(
                    "  {} · {}",
                    terminal_safe(&failure.target),
                    terminal_safe(&failure.message)
                ),
                detail::detail_width(output_width),
            );
        }
    }
    if !commands.is_empty() {
        output.push('\n');
        for (label, command) in commands {
            detail::push_tail_styled_line(
                &mut output,
                &format!("{label}: {command}"),
                style,
                TextStyle::Muted,
                detail::detail_width(output_width),
            );
        }
    }
    output
}

pub(super) fn table_fits(
    records: &[AccountCapacityRecord],
    options: &AccountRenderOptions<'_>,
) -> bool {
    let output_width = options.output_width;
    let AccountOutputWidth::Bounded(terminal_width) = output_width else {
        return !matches!(output_width, AccountOutputWidth::Unknown);
    };
    let table = table::render_table_records(
        records,
        options.query,
        PresentationStyle::Plain,
        options.glyph_profile,
    );
    let tail = table_tail_lines(records, options);
    table
        .lines()
        .chain(tail.iter().map(String::as_str))
        .all(|line| {
            cell_width(line).is_ok_and(|line_width| line_width <= usize::from(terminal_width.get()))
        })
}

pub(super) fn table_tail_lines(
    records: &[AccountCapacityRecord],
    options: &AccountRenderOptions<'_>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if !options.warnings.is_empty() {
        lines.push("Refresh warnings".to_owned());
        lines.extend(
            options
                .warnings
                .iter()
                .map(|warning| format!("  {}", terminal_safe(warning.message()))),
        );
    }
    if !options.failures.is_empty() {
        lines.push("Refresh failures".to_owned());
        lines.extend(options.failures.iter().map(|failure| {
            format!(
                "  {} · {}",
                terminal_safe(&failure.target),
                terminal_safe(&failure.message)
            )
        }));
    }
    lines.push(format!(
        "Detail: {}",
        common::account_command(options.query, "--detail")
    ));
    if !options.refreshed && records.iter().any(|record| record.report.is_none()) {
        lines.push(format!(
            "Refresh: {}",
            common::account_command(options.query, "--refresh")
        ));
    }
    if !options.failures.is_empty() {
        lines.push(format!(
            "Retry: {}",
            common::account_command(options.query, "--refresh")
        ));
    }
    lines
}
