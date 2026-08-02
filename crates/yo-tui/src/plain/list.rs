use std::{error::Error, fmt, num::NonZeroU16};

use unicode_segmentation::UnicodeSegmentation;

use crate::surface::{GraphemeError, cell_width};

/// Whether plain output may adapt to a known terminal width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputWidth {
    /// Preserve one deterministic row per item for pipes and files.
    Unbounded,
    /// Adapt presentation to this many terminal cells.
    Bounded(NonZeroU16),
}

/// How a column participates in responsive layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnBehavior {
    /// Keep the column in the primary row while a table remains viable.
    Pinned,
    /// Move the whole priority group below each row when space is scarce.
    Collapsible {
        priority: u8,
        continuation: ContinuationLayout,
    },
}

/// How a collapsed column is presented below its primary row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationLayout {
    /// Pack the complete heading/value pair beside other short pairs.
    Flow,
    /// Give the heading and its wrapped value an independent block.
    Block,
}

/// One semantic column in a plain responsive list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Column<'a> {
    pub heading: &'a str,
    pub behavior: ColumnBehavior,
}

/// Optional terminal decoration for semantic column headings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HeadingStyle {
    #[default]
    Plain,
    BoldAnsi,
}

/// Presentation settings shared by all rows in one plain list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListSpec<'a> {
    pub columns: &'a [Column<'a>],
    pub gap: NonZeroU16,
    pub heading_style: HeadingStyle,
}

/// Why a responsive plain list could not be rendered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListError {
    ColumnCount { expected: usize, actual: usize },
    InvalidText(GraphemeError),
    GraphemeExceedsWidth { grapheme_width: usize, width: usize },
}

impl fmt::Display for ListError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => {
                write!(
                    formatter,
                    "list row has {actual} columns; expected {expected}"
                )
            },
            Self::InvalidText(error) => {
                write!(formatter, "list text is not terminal-safe: {error}")
            },
            Self::GraphemeExceedsWidth {
                grapheme_width,
                width,
            } => write!(
                formatter,
                "a {grapheme_width}-cell grapheme cannot fit the {width}-cell output width"
            ),
        }
    }
}

impl Error for ListError {}

impl From<GraphemeError> for ListError {
    fn from(value: GraphemeError) -> Self {
        Self::InvalidText(value)
    }
}

/// Renders rows as a stable table or a width-aware terminal list.
pub fn render_list(
    spec: ListSpec<'_>,
    rows: &[Vec<String>],
    output_width: OutputWidth,
) -> Result<String, ListError> {
    validate_rows(spec.columns, rows)?;
    if rows.is_empty() {
        return Ok(String::new());
    }

    let natural_widths = natural_widths(spec.columns, rows)?;
    let mut visible = vec![true; spec.columns.len()];
    if let OutputWidth::Bounded(width) = output_width {
        collapse_to_fit(
            spec.columns,
            &natural_widths,
            usize::from(spec.gap.get()),
            usize::from(width.get()),
            &mut visible,
        );
        if table_width(&natural_widths, &visible, usize::from(spec.gap.get()))
            > usize::from(width.get())
        {
            return render_vertical(
                spec.columns,
                rows,
                usize::from(width.get()),
                spec.heading_style,
            );
        }
    }

    render_table(spec, rows, &natural_widths, &visible, output_width)
}

fn validate_rows(columns: &[Column<'_>], rows: &[Vec<String>]) -> Result<(), ListError> {
    for row in rows {
        if row.len() != columns.len() {
            return Err(ListError::ColumnCount {
                expected: columns.len(),
                actual: row.len(),
            });
        }
    }
    Ok(())
}

fn natural_widths(columns: &[Column<'_>], rows: &[Vec<String>]) -> Result<Vec<usize>, ListError> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            rows.iter()
                .map(|row| cell_width(&row[index]).map_err(ListError::from))
                .chain(std::iter::once(
                    cell_width(column.heading).map_err(ListError::from),
                ))
                .collect::<Result<Vec<_>, _>>()
                .map(|widths| widths.into_iter().max().unwrap_or(0))
        })
        .collect()
}

fn collapse_to_fit(
    columns: &[Column<'_>],
    widths: &[usize],
    gap: usize,
    available: usize,
    visible: &mut [bool],
) {
    while table_width(widths, visible, gap) > available {
        let next = columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| match column.behavior {
                ColumnBehavior::Collapsible { priority, .. } if visible[index] => Some(priority),
                ColumnBehavior::Pinned | ColumnBehavior::Collapsible { .. } => None,
            })
            .min();
        let Some(priority) = next else {
            break;
        };
        for (index, column) in columns.iter().enumerate() {
            if matches!(
                column.behavior,
                ColumnBehavior::Collapsible {
                    priority: candidate,
                    ..
                } if candidate == priority
            ) {
                visible[index] = false;
            }
        }
    }
}

fn table_width(widths: &[usize], visible: &[bool], gap: usize) -> usize {
    let mut count = 0_usize;
    let values = widths
        .iter()
        .zip(visible)
        .filter_map(|(width, visible)| visible.then_some(*width))
        .inspect(|_| count += 1)
        .sum::<usize>();
    values + gap.saturating_mul(count.saturating_sub(1))
}

fn render_table(
    spec: ListSpec<'_>,
    rows: &[Vec<String>],
    widths: &[usize],
    visible: &[bool],
    output_width: OutputWidth,
) -> Result<String, ListError> {
    let gap = usize::from(spec.gap.get());
    let mut output = render_inline(
        spec.columns.iter().map(|column| column.heading),
        widths,
        visible,
        gap,
        spec.heading_style,
    )?;
    output.push('\n');
    let folded = visible.iter().any(|value| !value);
    let bounded = match output_width {
        OutputWidth::Bounded(width) => Some(usize::from(width.get())),
        OutputWidth::Unbounded => None,
    };

    for (row_index, row) in rows.iter().enumerate() {
        output.push_str(&render_inline(
            row.iter().map(String::as_str),
            widths,
            visible,
            gap,
            HeadingStyle::Plain,
        )?);
        output.push('\n');
        if folded {
            render_folded(
                spec.columns,
                row,
                visible,
                bounded,
                gap,
                spec.heading_style,
                &mut output,
            )?;
            if row_index + 1 < rows.len() {
                output.push('\n');
            }
        }
    }
    Ok(output)
}

fn render_inline<'a>(
    values: impl Iterator<Item = &'a str>,
    widths: &[usize],
    visible: &[bool],
    gap: usize,
    heading_style: HeadingStyle,
) -> Result<String, ListError> {
    let selected = values
        .enumerate()
        .filter_map(|(index, value)| visible[index].then_some((index, value)))
        .collect::<Vec<_>>();
    let mut output = String::new();
    for (position, (index, value)) in selected.iter().enumerate() {
        push_heading(&mut output, value, heading_style);
        if position + 1 < selected.len() {
            let padding = widths[*index].saturating_sub(cell_width(value)?);
            output.push_str(&" ".repeat(padding + gap));
        }
    }
    Ok(output)
}

fn render_folded(
    columns: &[Column<'_>],
    row: &[String],
    visible: &[bool],
    bounded: Option<usize>,
    gap: usize,
    heading_style: HeadingStyle,
    output: &mut String,
) -> Result<(), ListError> {
    let available = bounded.expect("folded columns exist only for bounded output");
    let mut flow_width = 0_usize;
    for (index, column) in columns.iter().enumerate() {
        if visible[index] || row[index].is_empty() {
            continue;
        }
        let continuation = match column.behavior {
            ColumnBehavior::Collapsible { continuation, .. } => continuation,
            ColumnBehavior::Pinned => continue,
        };
        match continuation {
            ContinuationLayout::Flow => {
                let pair_width = cell_width(column.heading)? + gap + cell_width(&row[index])?;
                if pair_width > available {
                    flush_flow_line(&mut flow_width, output);
                    render_labeled_value(
                        column.heading,
                        &row[index],
                        bounded,
                        heading_style,
                        output,
                    )?;
                    continue;
                }
                let separator = usize::from(flow_width > 0) * gap;
                if flow_width > 0 && flow_width + separator + pair_width > available {
                    flush_flow_line(&mut flow_width, output);
                }
                if flow_width > 0 {
                    output.push_str(&" ".repeat(gap));
                    flow_width += gap;
                }
                push_heading(output, column.heading, heading_style);
                output.push_str(&" ".repeat(gap));
                output.push_str(&row[index]);
                flow_width += pair_width;
            },
            ContinuationLayout::Block => {
                flush_flow_line(&mut flow_width, output);
                render_block_value(
                    column.heading,
                    &row[index],
                    available,
                    gap,
                    heading_style,
                    output,
                )?;
            },
        }
    }
    flush_flow_line(&mut flow_width, output);
    Ok(())
}

fn render_block_value(
    label: &str,
    value: &str,
    available: usize,
    gap: usize,
    heading_style: HeadingStyle,
    output: &mut String,
) -> Result<(), ListError> {
    let pair_width = cell_width(label)? + gap + cell_width(value)?;
    if pair_width <= available {
        push_heading(output, label, heading_style);
        output.push_str(&" ".repeat(gap));
        output.push_str(value);
        output.push('\n');
        return Ok(());
    }
    render_labeled_value(label, value, Some(available), heading_style, output)
}

fn flush_flow_line(flow_width: &mut usize, output: &mut String) {
    if *flow_width > 0 {
        output.push('\n');
        *flow_width = 0;
    }
}

fn render_vertical(
    columns: &[Column<'_>],
    rows: &[Vec<String>],
    width: usize,
    heading_style: HeadingStyle,
) -> Result<String, ListError> {
    let mut output = String::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (column, value) in columns.iter().zip(row) {
            if !value.is_empty() {
                render_labeled_value(
                    column.heading,
                    value,
                    Some(width),
                    heading_style,
                    &mut output,
                )?;
            }
        }
        if row_index + 1 < rows.len() {
            output.push('\n');
        }
    }
    Ok(output)
}

fn render_labeled_value(
    label: &str,
    value: &str,
    bounded: Option<usize>,
    heading_style: HeadingStyle,
    output: &mut String,
) -> Result<(), ListError> {
    let label_width = bounded;
    for line in wrap(label, label_width)? {
        push_heading(output, &line, heading_style);
        output.push('\n');
    }
    let value_indent = bounded.map_or(2, |width| 2.min(width.saturating_sub(1)));
    let line_width = bounded.map(|width| width.saturating_sub(value_indent).max(1));
    for line in wrap(value, line_width)? {
        output.push_str(&" ".repeat(value_indent));
        output.push_str(&line);
        output.push('\n');
    }
    Ok(())
}

fn push_heading(output: &mut String, value: &str, style: HeadingStyle) {
    match style {
        HeadingStyle::Plain => output.push_str(value),
        HeadingStyle::BoldAnsi => {
            output.push_str("\u{1b}[1m");
            output.push_str(value);
            output.push_str("\u{1b}[0m");
        },
    }
}

fn wrap(value: &str, width: Option<usize>) -> Result<Vec<String>, ListError> {
    let Some(width) = width else {
        return Ok(vec![value.to_owned()]);
    };
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0_usize;
    for grapheme in value.graphemes(true) {
        let grapheme_width = match cell_width(grapheme) {
            Ok(width) => width,
            Err(GraphemeError::ZeroWidth) => 0,
            Err(error) => return Err(error.into()),
        };
        if grapheme_width > width {
            return Err(ListError::GraphemeExceedsWidth {
                grapheme_width,
                width,
            });
        }
        if !line.is_empty() && used + grapheme_width > width {
            lines.push(std::mem::take(&mut line));
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

#[cfg(test)]
mod tests;
