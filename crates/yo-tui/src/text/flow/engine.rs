//! Grapheme scanning and physical-cell placement for text flow.

use std::{collections::VecDeque, iter, num::NonZeroU16};

use unicode_segmentation::UnicodeSegmentation;

use super::{
    PositionedGrapheme, TextFlowError,
    display::{control_notation, tab_spaces},
};
use crate::surface::{Grapheme, Point};

pub(super) struct RawFlow {
    pub(super) glyphs: Vec<PositionedGrapheme>,
    pub(super) cursor: Option<Point>,
    pub(super) content_height: u16,
    pub(super) skipped_rows: usize,
}

enum GlyphBuffer {
    Full(Vec<PositionedGrapheme>),
    Tail {
        rows: VecDeque<(usize, Vec<PositionedGrapheme>)>,
        limit: NonZeroU16,
    },
}

trait FlowSink {
    fn hard_break(&mut self, _row: usize, _next_byte: usize) {}

    fn next_row(&self, y: &mut usize) -> Result<(), TextFlowError> {
        *y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
        Ok(())
    }

    fn push(
        &mut self,
        row: usize,
        byte_index: usize,
        x: u16,
        grapheme: Grapheme,
    ) -> Result<(), TextFlowError>;
}

impl FlowSink for GlyphBuffer {
    fn next_row(&self, y: &mut usize) -> Result<(), TextFlowError> {
        *y = y
            .checked_add(1)
            .filter(|next| matches!(self, Self::Tail { .. }) || *next <= usize::from(u16::MAX))
            .ok_or(TextFlowError::HeightOverflow)?;
        Ok(())
    }

    fn push(
        &mut self,
        row: usize,
        byte_index: usize,
        x: u16,
        grapheme: Grapheme,
    ) -> Result<(), TextFlowError> {
        match self {
            Self::Full(glyphs) => glyphs.push(PositionedGrapheme {
                byte_index,
                point: Point::new(
                    x,
                    u16::try_from(row).map_err(|_| TextFlowError::HeightOverflow)?,
                ),
                grapheme,
            }),
            Self::Tail { rows, limit } => {
                let first = row.saturating_sub(usize::from(limit.get()) - 1);
                let mut reusable = None;
                while rows.front().is_some_and(|(row, _)| *row < first) {
                    let (_, mut glyphs) = rows.pop_front().expect("the oldest row was checked");
                    glyphs.clear();
                    reusable = Some(glyphs);
                }
                if rows.back().is_none_or(|(last, _)| *last != row) {
                    rows.push_back((row, reusable.unwrap_or_default()));
                }
                rows.back_mut()
                    .expect("current row was inserted")
                    .1
                    .push(PositionedGrapheme {
                        byte_index,
                        point: Point::new(x, 0),
                        grapheme,
                    });
            },
        }
        Ok(())
    }
}

impl GlyphBuffer {
    fn finish(self, height: usize) -> Result<(Vec<PositionedGrapheme>, u16, usize), TextFlowError> {
        match self {
            Self::Full(glyphs) => Ok((
                glyphs,
                u16::try_from(height).map_err(|_| TextFlowError::HeightOverflow)?,
                0,
            )),
            Self::Tail { rows, limit } => {
                let retained = height.min(usize::from(limit.get()));
                let skipped = height - retained;
                let glyphs = rows
                    .into_iter()
                    .filter(|(row, _)| *row >= skipped)
                    .flat_map(|(row, glyphs)| {
                        glyphs.into_iter().map(move |mut glyph| {
                            glyph.point.y = (row - skipped) as u16;
                            glyph
                        })
                    })
                    .collect();
                Ok((glyphs, retained as u16, skipped))
            },
        }
    }
}

pub(super) fn flow_tail(
    text: &str,
    width: NonZeroU16,
    rows: NonZeroU16,
) -> Result<RawFlow, TextFlowError> {
    flow_with_buffer(
        text,
        None,
        width,
        GlyphBuffer::Tail {
            rows: VecDeque::new(),
            limit: rows,
        },
    )
}

pub(super) fn flow(
    text: &str,
    source_cursor: Option<usize>,
    width: NonZeroU16,
) -> Result<RawFlow, TextFlowError> {
    flow_with_buffer(text, source_cursor, width, GlyphBuffer::Full(Vec::new()))
}

fn flow_with_buffer(
    text: &str,
    source_cursor: Option<usize>,
    width: NonZeroU16,
    mut glyphs: GlyphBuffer,
) -> Result<RawFlow, TextFlowError> {
    let (content_height, cursor_point) = scan(text, source_cursor, width, &mut glyphs)?;
    let (glyphs, content_height, skipped_rows) = glyphs.finish(content_height)?;

    Ok(RawFlow {
        glyphs,
        cursor: cursor_point,
        content_height,
        skipped_rows,
    })
}

fn scan(
    text: &str,
    source_cursor: Option<usize>,
    width: NonZeroU16,
    glyphs: &mut impl FlowSink,
) -> Result<(usize, Option<Point>), TextFlowError> {
    let width = width.get();
    let mut x = 0_u16;
    let mut y = 0_usize;
    let mut cursor_point = None;

    for (byte_index, text) in text.grapheme_indices(true) {
        if is_hard_break(text) {
            if source_cursor == Some(byte_index) {
                cursor_point = Some(normalized_point(x, y, width)?);
            }
            x = 0;
            glyphs.next_row(&mut y)?;
            glyphs.hard_break(y, byte_index + text.len());
            continue;
        }

        if text == "\t" {
            if x == width {
                x = 0;
                glyphs.next_row(&mut y)?;
            }
            let spaces = tab_spaces(x);
            for offset in 0..spaces {
                place_grapheme(
                    Grapheme::try_from(" ").expect("ASCII space is renderable"),
                    byte_index,
                    source_cursor,
                    offset == 0,
                    width,
                    &mut x,
                    &mut y,
                    &mut cursor_point,
                    glyphs,
                )?;
            }
            continue;
        }

        if let Some(notation) = control_notation(text) {
            for (offset, character) in notation.chars().enumerate() {
                let mut encoded = [0; 4];
                let character = character.encode_utf8(&mut encoded);
                place_grapheme(
                    Grapheme::try_from(&*character).expect("ASCII control notation is renderable"),
                    byte_index,
                    source_cursor,
                    offset == 0,
                    width,
                    &mut x,
                    &mut y,
                    &mut cursor_point,
                    glyphs,
                )?;
            }
            continue;
        }

        let grapheme = Grapheme::try_from(text)
            .map_err(|cause| TextFlowError::UnrenderableGrapheme { byte_index, cause })?;
        place_grapheme(
            grapheme,
            byte_index,
            source_cursor,
            true,
            width,
            &mut x,
            &mut y,
            &mut cursor_point,
            glyphs,
        )?;
    }

    if source_cursor.is_some() && cursor_point.is_none() {
        cursor_point = Some(normalized_point(x, y, width)?);
    }
    let content_height = if text.is_empty() {
        0
    } else {
        y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?
    };
    Ok((content_height, cursor_point))
}

/// Display text indexed at sparse row checkpoints, independent of surface height.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TextPages {
    text: String,
    checkpoints: Vec<usize>,
    source_rows: Vec<usize>,
    rows: usize,
    source_map: Vec<(usize, usize)>,
}

impl TextPages {
    const STRIDE: usize = 128;

    pub(crate) fn new(source: &str, width: NonZeroU16) -> Result<Self, TextFlowError> {
        let mut pages = Self {
            text: String::new(),
            checkpoints: vec![0],
            source_rows: vec![0],
            rows: 0,
            source_map: vec![(0, 0)],
        };
        let (height, _) = scan(source, None, width, &mut pages)?;
        if height > 0 {
            pages.advance_to(height - 1);
        }
        pages.rows = height;
        Ok(pages)
    }

    /// Read-only pages with explicit ASCII fallback and original source positions.
    pub(crate) fn with_escaped_fallback(
        source: &str,
        width: NonZeroU16,
        notice: &str,
    ) -> Result<Self, TextFlowError> {
        Self::new(source, width).or_else(|_| {
            let mut shown = format!("{notice}\n");
            let mut escapes = vec![(0..shown.len(), 0..0)];
            for (offset, character) in source.char_indices() {
                let start = shown.len();
                if character == '\n' {
                    shown.push(character);
                } else {
                    shown.extend(character.escape_default());
                }
                let end = offset + character.len_utf8();
                if shown[start..] != source[offset..end] {
                    escapes.push((start..shown.len(), offset..end));
                }
            }
            let mut pages = Self::new(&shown, width)?;
            for offset in &mut pages.source_rows {
                let next = escapes.partition_point(|(display, _)| display.start <= *offset);
                let (display, original) = &escapes[next - 1];
                *offset = if display.contains(offset) {
                    original.start
                } else {
                    original.end + offset.saturating_sub(display.end)
                };
            }
            Ok(pages)
        })
    }

    pub(crate) const fn row_count(&self) -> usize {
        self.rows
    }

    /// Original source position for a projected display glyph. Only discontinuities
    /// (wrapping, expanded controls, omitted prose spaces) need a mapping entry.
    pub(crate) fn source_byte_at(&self, display_byte: usize) -> usize {
        let next = self
            .source_map
            .partition_point(|(display, _)| *display <= display_byte);
        let (display, source) = self.source_map[next.saturating_sub(1)];
        source + display_byte.saturating_sub(display)
    }

    pub(crate) fn window_offset(&self, row: usize) -> usize {
        if row >= self.rows {
            self.text.len()
        } else {
            self.row_offset(row)
        }
    }

    pub(crate) fn prose(
        source: &str,
        width: NonZeroU16,
        preserve_indent: bool,
        preserve_spaces: bool,
    ) -> Result<Self, TextFlowError> {
        let mut pages = Self {
            text: String::new(),
            checkpoints: vec![0],
            source_rows: vec![0],
            rows: 0,
            source_map: vec![(0, 0)],
        };
        let mut row = 0_usize;
        let mut offset = 0_usize;
        for (line_index, line) in source.split('\n').enumerate() {
            if line_index > 0 {
                row = row.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
                pages.hard_break(row, offset);
            }
            let mut input = ExpandedLine::new(line, width);
            let continuation_indent = if preserve_spaces {
                let mut indent = 0_usize;
                for glyph in input.clone() {
                    let (_, glyph) = glyph.map_err(|error| offset_error(error, offset))?;
                    if !glyph.as_str().chars().all(char::is_whitespace) {
                        break;
                    }
                    indent += usize::from(glyph.width().get());
                }
                indent.min(usize::from(width.get() / 4)) as u16
            } else {
                0
            };
            let mut column = 0_u16;
            let mut leading = true;
            let mut previous_space = true;
            while let Some(glyph) = input.next() {
                let (byte, glyph) = glyph.map_err(|error| offset_error(error, offset))?;
                let is_space = glyph.as_str().chars().all(char::is_whitespace);
                if !is_space && previous_space && column > 0 && !(preserve_indent && leading) {
                    let mut word_width = usize::from(glyph.width().get());
                    for next in input.clone() {
                        let (_, next) = next.map_err(|error| offset_error(error, offset))?;
                        if next.as_str().chars().all(char::is_whitespace) {
                            break;
                        }
                        word_width = word_width
                            .checked_add(usize::from(next.width().get()))
                            .ok_or(TextFlowError::HeightOverflow)?;
                    }
                    if word_width <= usize::from(width.get() - continuation_indent)
                        && word_width > usize::from(width.get() - column)
                    {
                        if !preserve_spaces {
                            pages.trim_row_spaces(row);
                        }
                        column = continuation_indent;
                        row = row.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
                    }
                }
                previous_space = is_space;
                if !preserve_spaces
                    && is_space
                    && (column == 0 || column == width.get())
                    && !(preserve_indent && leading)
                {
                    continue;
                }
                leading &= is_space;
                if column
                    .checked_add(glyph.width().get())
                    .is_none_or(|end| end > width.get())
                {
                    column = continuation_indent;
                    row = row.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
                }
                pages.advance_to(row);
                if pages.text.ends_with('\n') || pages.text.is_empty() {
                    pages.text.extend(iter::repeat_n(' ', usize::from(column)));
                }
                let glyph_width = glyph.width().get();
                pages.push(row, offset + byte, column, glyph)?;
                column += glyph_width;
            }
            offset += line.len() + 1;
        }
        let height = if source.is_empty() {
            0
        } else {
            row.checked_add(1).ok_or(TextFlowError::HeightOverflow)?
        };
        if height > 0 {
            pages.advance_to(height - 1);
        }
        pages.rows = height;
        Ok(pages)
    }

    fn trim_row_spaces(&mut self, row: usize) {
        if self.rows != row {
            return;
        }
        let beginning = self.text.rfind('\n').map_or(0, |offset| offset + 1);
        let end = beginning
            + self.text[beginning..]
                .trim_end_matches(char::is_whitespace)
                .len();
        self.text.truncate(end);
        let retained = self
            .source_map
            .partition_point(|(display, _)| *display <= end);
        self.source_map.truncate(retained);
    }

    /// Original byte offset at the first grapheme of a display row, including empty rows.
    pub(crate) fn source_offset(&self, row: usize) -> usize {
        self.source_rows[row.min(self.source_rows.len() - 1)]
    }

    /// Display row containing an original byte offset; expanded tabs use their first row.
    pub(crate) fn row_for_source(&self, byte: usize) -> usize {
        let next = self.source_rows.partition_point(|offset| *offset < byte);
        if self.source_rows.get(next) == Some(&byte) {
            next
        } else {
            next.saturating_sub(1)
        }
    }

    /// A bounded display window; source offsets belong to the original tool output.
    pub(crate) fn window(&self, start: usize, rows: NonZeroU16) -> &str {
        if start >= self.rows {
            return "";
        }
        let begin = self.row_offset(start);
        let end_row = start.saturating_add(usize::from(rows.get()));
        let end = if end_row >= self.rows {
            self.text.len()
        } else {
            // Exclude the separator belonging to the following page.
            self.row_offset(end_row) - 1
        };
        &self.text[begin..end]
    }

    fn row_offset(&self, row: usize) -> usize {
        let checkpoint = row / Self::STRIDE;
        let mut offset = self.checkpoints[checkpoint];
        for _ in 0..row % Self::STRIDE {
            offset += self.text[offset..].find('\n').expect("indexed display row") + 1;
        }
        offset
    }

    fn advance_to(&mut self, row: usize) {
        while self.rows < row {
            self.text.push('\n');
            self.rows += 1;
            if self.rows.is_multiple_of(Self::STRIDE) {
                self.checkpoints.push(self.text.len());
            }
        }
    }
}

impl FlowSink for TextPages {
    fn hard_break(&mut self, row: usize, next_byte: usize) {
        self.source_rows.resize(row + 1, next_byte);
    }

    fn push(
        &mut self,
        row: usize,
        byte_index: usize,
        _: u16,
        grapheme: Grapheme,
    ) -> Result<(), TextFlowError> {
        self.source_rows.resize(row + 1, byte_index);
        self.advance_to(row);
        let display = self.text.len();
        if self.source_byte_at(display) != byte_index {
            self.source_map.push((display, byte_index));
        }
        self.text.push_str(grapheme.as_str());
        Ok(())
    }
}

fn offset_error(error: TextFlowError, offset: usize) -> TextFlowError {
    match error {
        TextFlowError::GraphemeTooWide { byte_index, width } => TextFlowError::GraphemeTooWide {
            byte_index: offset + byte_index,
            width,
        },
        TextFlowError::UnrenderableGrapheme { byte_index, cause } => {
            TextFlowError::UnrenderableGrapheme {
                byte_index: offset + byte_index,
                cause,
            }
        },
        error => error,
    }
}

/// The literal scanner's glyph projection without retaining a line's cell array.
/// A clone supports bounded word lookahead; tabs/control notation use at most a
/// handful of pending glyphs, even when one source line wraps past 65,535 rows.
#[derive(Clone)]
struct ExpandedLine<'a> {
    input: unicode_segmentation::GraphemeIndices<'a>,
    pending: VecDeque<(usize, Grapheme)>,
    column: u16,
    width: NonZeroU16,
}

impl<'a> ExpandedLine<'a> {
    fn new(source: &'a str, width: NonZeroU16) -> Self {
        Self {
            input: source.grapheme_indices(true),
            pending: VecDeque::new(),
            column: 0,
            width,
        }
    }
}

impl Iterator for ExpandedLine<'_> {
    type Item = Result<(usize, Grapheme), TextFlowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((byte, glyph)) = self.pending.pop_front() {
                if self
                    .column
                    .checked_add(glyph.width().get())
                    .is_none_or(|end| end > self.width.get())
                {
                    self.column = 0;
                }
                self.column += glyph.width().get();
                return Some(Ok((byte, glyph)));
            }
            let (byte, text) = self.input.next()?;
            if is_hard_break(text) {
                self.column = 0;
                continue;
            }
            if text == "\t" {
                if self.column == self.width.get() {
                    self.column = 0;
                }
                for _ in 0..tab_spaces(self.column) {
                    self.pending
                        .push_back((byte, Grapheme::try_from(" ").expect("ASCII space")));
                }
            } else if let Some(notation) = control_notation(text) {
                for character in notation.chars() {
                    let mut encoded = [0; 4];
                    let text = character.encode_utf8(&mut encoded);
                    self.pending.push_back((
                        byte,
                        Grapheme::try_from(&*text).expect("ASCII control notation"),
                    ));
                }
            } else {
                let glyph = match Grapheme::try_from(text) {
                    Ok(glyph) => glyph,
                    Err(cause) => {
                        return Some(Err(TextFlowError::UnrenderableGrapheme {
                            byte_index: byte,
                            cause,
                        }));
                    },
                };
                if glyph.width() > self.width {
                    return Some(Err(TextFlowError::GraphemeTooWide {
                        byte_index: byte,
                        width: glyph.width(),
                    }));
                }
                self.pending.push_back((byte, glyph));
            }
        }
    }
}

pub(super) fn validate_cursor(text: &str, cursor: usize) -> Result<(), TextFlowError> {
    if cursor > text.len() {
        return Err(TextFlowError::CursorOutOfBounds);
    }
    if cursor == text.len()
        || text
            .grapheme_indices(true)
            .any(|(byte_index, _)| byte_index == cursor)
    {
        Ok(())
    } else {
        Err(TextFlowError::CursorNotOnGraphemeBoundary)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the helper advances one explicit text-flow cursor and output"
)]
fn place_grapheme(
    grapheme: Grapheme,
    byte_index: usize,
    source_cursor: Option<usize>,
    marks_source_start: bool,
    width: u16,
    x: &mut u16,
    y: &mut usize,
    cursor: &mut Option<Point>,
    glyphs: &mut impl FlowSink,
) -> Result<(), TextFlowError> {
    let grapheme_width = grapheme.width();
    if grapheme_width.get() > width {
        return Err(TextFlowError::GraphemeTooWide {
            byte_index,
            width: grapheme_width,
        });
    }

    if x.checked_add(grapheme_width.get())
        .is_none_or(|end| end > width)
    {
        *x = 0;
        glyphs.next_row(y)?;
    }
    if marks_source_start && source_cursor == Some(byte_index) {
        *cursor = Some(normalized_point(*x, *y, width)?);
    }

    glyphs.push(*y, byte_index, *x, grapheme)?;
    *x += grapheme_width.get();
    Ok(())
}

fn normalized_point(x: u16, y: usize, width: u16) -> Result<Point, TextFlowError> {
    let y = u16::try_from(y).map_err(|_| TextFlowError::HeightOverflow)?;
    if x < width {
        return Ok(Point::new(x, y));
    }

    Ok(Point::new(
        0,
        y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?,
    ))
}

fn is_hard_break(text: &str) -> bool {
    matches!(text, "\n" | "\r" | "\r\n")
}
