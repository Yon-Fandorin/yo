use std::num::NonZeroU16;

use yo_core::MessageContent;

use super::{
    TranscriptBody, TranscriptLayoutConfig, TranscriptMeasureError, TranscriptPhase,
    TranscriptSlice, activity, marker_glyphs, measure_error, separator_height,
};
use crate::text::flow::{TextPages, flow_text};

pub(crate) fn plain_output(
    transcript: TranscriptSlice<'_>,
    config: &TranscriptLayoutConfig,
) -> Result<Option<String>, TranscriptMeasureError> {
    config
        .validate_for_width(u16::MAX)
        .map_err(TranscriptMeasureError::InvalidConfig)?;
    let mut output = String::new();
    let mut row = 0_usize;
    let mut column = 0_u16;
    let mut base = 0_usize;
    let mut has_visible_predecessor = transcript.has_visible_predecessor();
    for item in transcript.items() {
        let TranscriptBody::Message(message) = item.body();
        if message.text().is_empty() && item.phase() == TranscriptPhase::Streaming {
            continue;
        }
        let width = NonZeroU16::new(u16::MAX - config.body_indent())
            .ok_or(TranscriptMeasureError::BodyWidthUnavailable)?;
        let width = config
            .max_body_width()
            .map_or(width, |maximum| maximum.min(width));
        let source = activity::model_source_text(message, width)
            .map_err(TranscriptMeasureError::Text)?
            .or_else(|| activity::tool_source_text(message))
            .or_else(|| {
                let end = message
                    .assistant_footer_start
                    .unwrap_or(message.text().len());
                message
                    .is_markdown()
                    .then(|| MessageContent::from_snapshot(&message.text()[..end]))
                    .flatten()
                    .map(|content| format!("{:#}{}", content.block, &message.text()[end..]))
            });
        let source = source.as_deref().unwrap_or(message.text());
        let pages = TextPages::new(source, width).map_err(TranscriptMeasureError::Text)?;
        if has_visible_predecessor {
            base += usize::from(separator_height(message.role()));
        }
        for marker in marker_glyphs(config.marker(message.role()), 0, message.role())
            .map_err(measure_error)?
        {
            append_gap(&mut output, &mut row, &mut column, marker.point.x, base);
            column += marker.grapheme.width().get();
            output.push_str(marker.grapheme.as_str());
        }
        let page_height = NonZeroU16::new(128).expect("nonzero export page");
        for start in (0..pages.row_count()).step_by(128) {
            let page = flow_text(pages.window(start, page_height), width)
                .map_err(TranscriptMeasureError::Text)?;
            for glyph in page.glyphs {
                append_gap(
                    &mut output,
                    &mut row,
                    &mut column,
                    config.body_indent() + glyph.point.x,
                    base + start + usize::from(glyph.point.y),
                );
                column += glyph.grapheme.width().get();
                output.push_str(glyph.grapheme.as_str());
            }
        }
        base += pages.row_count().max(1);
        has_visible_predecessor = true;
    }
    if output.is_empty() {
        Ok(None)
    } else {
        output.push('\n');
        Ok(Some(output))
    }
}

fn append_gap(output: &mut String, row: &mut usize, column: &mut u16, x: u16, y: usize) {
    while *row < y {
        output.push('\n');
        *row += 1;
        *column = 0;
    }
    debug_assert!(
        x >= *column,
        "prepared transcript glyphs must be ordered and non-overlapping"
    );
    output.extend(std::iter::repeat_n(' ', usize::from(x - *column)));
    *column = x;
}
