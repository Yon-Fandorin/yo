//! Width-dependent table presentation; cells retain their inline decorations.

use std::mem::take;

use pulldown_cmark::Alignment;

use super::{
    Attributes, Block, BlockFormat, Decoration, NonZeroU16, Role, TextFlowError, flow_prose,
    flow_text,
};

pub(super) struct Table {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Block>>,
    current: Vec<Block>,
    prefix: String,
    gap: bool,
}

struct Cell {
    block: Block,
    width: u16,
}

impl Table {
    pub(super) fn new(alignments: Vec<Alignment>, prefix: String, gap: bool) -> Self {
        Self {
            alignments,
            rows: Vec::new(),
            current: Vec::new(),
            prefix,
            gap,
        }
    }

    pub(super) fn push_cell(&mut self, cell: Block) {
        self.current.push(cell);
    }

    pub(super) fn finish_row(&mut self) {
        self.rows.push(take(&mut self.current));
    }

    pub(super) fn into_blocks(self, width: NonZeroU16) -> Result<Vec<Block>, TextFlowError> {
        let columns = self.alignments.len();
        if columns == 0 {
            return Ok(Vec::new());
        }
        let rows = self
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(normalize)
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut widths = vec![1_u16; columns];
        for row in &rows {
            for (index, cell) in row.iter().enumerate().take(columns) {
                widths[index] = widths[index].max(cell.width);
            }
        }
        let indent = self
            .prefix
            .len()
            .min(usize::from(width.get().saturating_sub(2)));
        let available = usize::from(width.get()) - indent;
        let natural = widths
            .iter()
            .map(|&width| usize::from(width))
            .sum::<usize>()
            + (columns - 1) * 3;
        // Keep a readable grid while columns can retain at least eight cells.
        // Only very narrow terminals use labeled records; never truncate values.
        let minimum: usize =
            widths.iter().map(|&w| usize::from(w.min(8))).sum::<usize>() + (columns - 1) * 3;
        let mut blocks = if natural <= available || minimum <= available {
            if natural > available {
                fit_widths(&mut widths, available - (columns - 1) * 3);
            }
            grid(&rows, &widths, &self.alignments)?
        } else {
            stacked(&rows, columns)
        };
        for (index, block) in blocks.iter_mut().enumerate() {
            block.prefix = if index == 0 {
                self.prefix.clone()
            } else {
                self.prefix
                    .chars()
                    .map(|c| if c == '>' { '>' } else { ' ' })
                    .collect()
            };
        }
        if let Some(first) = blocks.first_mut() {
            first.gap = self.gap;
        }
        Ok(blocks)
    }
}

// Find a common ceiling instead of rescanning every column for each removed cell.
// Earlier columns keep the remainder, matching the existing last-column tie break.
fn fit_widths(widths: &mut [u16], available: usize) {
    let mut low = 1;
    let mut high = widths.iter().copied().max().unwrap_or(1);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let used: usize = widths
            .iter()
            .map(|&width| usize::from(width.min(middle)))
            .sum();
        if used <= available {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let used: usize = widths
        .iter()
        .map(|&width| usize::from(width.min(low)))
        .sum();
    let mut remaining = available - used;
    for width in widths {
        let extra = usize::from(*width > low && remaining > 0);
        *width = (*width).min(low) + extra as u16;
        remaining -= extra;
    }
}

// Normalize tabs and controls within each cell before column alignment. Flow's
// source offsets preserve styles even when one source grapheme expands to cells.
fn normalize(block: Block) -> Result<Cell, TextFlowError> {
    let flow = flow_text(&block.text, NonZeroU16::new(u16::MAX).unwrap())?;
    let mut normalized = Block::default();
    let mut width = 0;
    let mut row = 0;
    let mut span = 0;
    for glyph in flow.glyphs {
        while row < glyph.point.y {
            append(&mut normalized, "\n", Decoration::default());
            row += 1;
        }
        while block
            .spans
            .get(span + 1)
            .is_some_and(|(start, _)| *start <= glyph.byte_index)
        {
            span += 1;
        }
        let decoration = block
            .spans
            .get(span)
            .map_or(Decoration::default(), |(_, decoration)| *decoration);
        append(&mut normalized, glyph.grapheme.as_str(), decoration);
        width = width.max(glyph.point.x + glyph.grapheme.width().get());
    }
    Ok(Cell {
        block: normalized,
        width,
    })
}

fn grid(
    rows: &[Vec<Cell>],
    widths: &[u16],
    alignments: &[Alignment],
) -> Result<Vec<Block>, TextFlowError> {
    let mut blocks = Vec::new();
    for (row_index, cells) in rows.iter().enumerate() {
        let wrapped = widths
            .iter()
            .enumerate()
            .map(|(index, &width)| {
                let Some(cell) = cells.get(index) else {
                    return Ok(vec![Block::default()]);
                };
                let flow = flow_prose(&cell.block.text, NonZeroU16::new(width).unwrap())?;
                let mut lines = vec![Block::default(); usize::from(flow.height.max(1))];
                let mut span = 0;
                for glyph in flow.glyphs {
                    while cell
                        .block
                        .spans
                        .get(span + 1)
                        .is_some_and(|(start, _)| *start <= glyph.byte_index)
                    {
                        span += 1;
                    }
                    let decoration = cell
                        .block
                        .spans
                        .get(span)
                        .map_or(Decoration::default(), |(_, d)| *d);
                    append(
                        &mut lines[usize::from(glyph.point.y)],
                        glyph.grapheme.as_str(),
                        decoration,
                    );
                }
                Ok(lines)
            })
            .collect::<Result<Vec<_>, TextFlowError>>()?;
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line in 0..height {
            let mut block = Block {
                format: BlockFormat::TableRow,
                ..Block::default()
            };
            for (index, &width) in widths.iter().enumerate() {
                if index > 0 {
                    append(&mut block, " │ ", Decoration::role(Role::Quote));
                }
                let cell = wrapped[index].get(line);
                let used = if let Some(cell) = cell {
                    flow_text(&cell.text, NonZeroU16::new(u16::MAX).unwrap())?
                        .glyphs
                        .iter()
                        .map(|g| g.grapheme.width().get())
                        .sum::<u16>()
                } else {
                    0
                };
                let padding = width.saturating_sub(used);
                let left = match alignments[index] {
                    Alignment::Right => padding,
                    Alignment::Center => padding / 2,
                    _ => 0,
                };
                append(
                    &mut block,
                    &" ".repeat(usize::from(left)),
                    Decoration::default(),
                );
                if let Some(cell) = cell {
                    append_block(&mut block, cell);
                }
                append(
                    &mut block,
                    &" ".repeat(usize::from(padding - left)),
                    Decoration::default(),
                );
            }
            blocks.push(block);
        }
        if row_index == 0 {
            let rule = widths
                .iter()
                .map(|&width| "─".repeat(usize::from(width)))
                .collect::<Vec<_>>()
                .join("─┼─");
            let mut block = Block {
                format: BlockFormat::TableRow,
                ..Block::default()
            };
            append(&mut block, &rule, Decoration::role(Role::Quote));
            blocks.push(block);
        }
    }
    Ok(blocks)
}

fn stacked(rows: &[Vec<Cell>], columns: usize) -> Vec<Block> {
    let Some(headers) = rows.first() else {
        return Vec::new();
    };
    if rows.len() == 1 {
        return headers.iter().map(|cell| cell.block.clone()).collect();
    }
    let mut blocks = Vec::new();
    for (row_index, row) in rows.iter().skip(1).enumerate() {
        for index in 0..columns {
            let mut block = Block {
                gap: row_index > 0 && index == 0,
                ..Block::default()
            };
            if let Some(header) = headers
                .get(index)
                .filter(|cell| !cell.block.text.is_empty())
            {
                append_block(&mut block, &header.block);
            } else {
                append(
                    &mut block,
                    &format!("Column {}", index + 1),
                    Decoration {
                        role: Role::Heading,
                        attributes: Attributes::BOLD,
                        hyperlink: None,
                    },
                );
            }
            append(&mut block, ": ", Decoration::role(Role::Quote));
            if let Some(cell) = row.get(index) {
                append_block(&mut block, &cell.block);
            }
            blocks.push(block);
        }
    }
    blocks
}

fn append_block(target: &mut Block, source: &Block) {
    if source.spans.is_empty() {
        append(target, &source.text, Decoration::default());
        return;
    }
    for (index, &(start, decoration)) in source.spans.iter().enumerate() {
        let end = source
            .spans
            .get(index + 1)
            .map_or(source.text.len(), |(start, _)| *start);
        append(target, &source.text[start..end], decoration);
    }
}

fn append(block: &mut Block, text: &str, decoration: Decoration) {
    if text.is_empty() {
        return;
    }
    if block
        .spans
        .last()
        .is_none_or(|(_, last)| *last != decoration)
    {
        block.spans.push((block.text.len(), decoration));
    }
    block.text.push_str(text);
}
