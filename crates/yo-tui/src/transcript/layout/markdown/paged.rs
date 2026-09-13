//! Parsed blocks retain display bytes and source spans, not a cell per glyph.
//! Only a viewport's rows acquire Surface coordinates and styled glyphs.

use super::{
    BlockFormat, CodeBlockKind, Decoration, Document, Event, Hyperlink, LinkResolver,
    MarkdownGlyph, NonZeroU16, Point, PreparedMarkdown, RasterImage, Role, Tag, TagEnd,
    TextFlowError, expand_blocks, flow_text, parse_document,
};
use crate::text::flow::TextPages;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PagedMarkdown {
    blocks: Vec<PagedBlock>,
    hyperlinks: Vec<Hyperlink>,
    pub(crate) height: usize,
    width: NonZeroU16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PagedBlock {
    start: usize,
    height: usize,
    pages: TextPages,
    spans: Vec<(usize, Decoration)>,
    prefix: String,
    indent: u16,
    body_width: NonZeroU16,
    format: BlockFormat,
    raster: Option<RasterImage>,
}

impl PagedMarkdown {
    pub(crate) fn raster_rows(&self) -> Vec<(usize, RasterImage)> {
        self.blocks
            .iter()
            .filter_map(|block| {
                let mut raster = block.raster.clone()?;
                raster.area.origin = Point::new(block.indent, 0);
                Some((block.start, raster))
            })
            .collect()
    }
    pub(crate) fn new(
        source: &str,
        width: NonZeroU16,
        show_images: bool,
        image_max_width: NonZeroU16,
        show_diagrams: bool,
        resolver: Option<&LinkResolver>,
        code_padding: u16,
    ) -> Result<Self, TextFlowError> {
        let document = parse_document(source, width, resolver, code_padding);
        Self::from_document(document, width, show_images, image_max_width, show_diagrams)
    }

    pub(crate) fn diff(
        source: &str,
        width: NonZeroU16,
        code_padding: u16,
    ) -> Result<Self, TextFlowError> {
        let mut document = Document {
            code_padding: code_padding.min(width.get().saturating_sub(2)),
            ..Document::default()
        };
        document.event(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(
            "diff".into(),
        ))));
        document.event(Event::Text(source.into()));
        document.event(Event::End(TagEnd::CodeBlock));
        document.flush();
        Self::from_document(
            document,
            width,
            false,
            NonZeroU16::new(64).expect("nonzero width"),
            false,
        )
    }

    fn from_document(
        document: Document,
        width: NonZeroU16,
        show_images: bool,
        image_max_width: NonZeroU16,
        show_diagrams: bool,
    ) -> Result<Self, TextFlowError> {
        let source = expand_blocks(
            document.blocks,
            width,
            show_images,
            image_max_width,
            show_diagrams,
        )?;
        let mut result = Self {
            blocks: Vec::new(),
            hyperlinks: document.hyperlink_targets,
            height: 0,
            width,
        };
        for block in source {
            if block.gap && result.height > 0 {
                result.height = result
                    .height
                    .checked_add(1)
                    .ok_or(TextFlowError::HeightOverflow)?;
            }
            let prefix: String = block
                .prefix
                .chars()
                .take(usize::from(width.get().saturating_sub(2)))
                .collect();
            let indent =
                u16::try_from(prefix.chars().count()).expect("single-cell Markdown prefixes");
            let padding = if block.format.is_code() {
                (width.get() - indent)
                    .saturating_sub(4)
                    .min(document.code_padding)
            } else {
                0
            };
            let body_width =
                NonZeroU16::new(width.get() - indent - padding).expect("reserved body cells");
            let text = if block.format.is_code() {
                block.text.strip_suffix('\n').unwrap_or(&block.text)
            } else {
                &block.text
            };
            let pages = if block.format.is_code() {
                TextPages::prose(text, body_width, true, true)?
            } else if block.format == BlockFormat::Prose {
                TextPages::prose(text, body_width, false, false)?
            } else {
                TextPages::new(text, body_width)?
            };
            let height = pages.row_count().max(1);
            let end = result
                .height
                .checked_add(height)
                .ok_or(TextFlowError::HeightOverflow)?;
            result.blocks.push(PagedBlock {
                start: result.height,
                height,
                pages,
                spans: block.spans,
                prefix,
                indent,
                body_width,
                format: block.format,
                raster: block.raster,
            });
            result.height = end;
        }
        Ok(result)
    }

    /// All text was validated during indexing. This bounded pass cannot overflow
    /// or encounter a new grapheme; styles and links resolve from original spans.
    pub(crate) fn window(&self, first: usize, height: NonZeroU16) -> PreparedMarkdown {
        let end = first.saturating_add(usize::from(height.get()));
        let mut result = PreparedMarkdown {
            glyphs: Vec::new(),
            row_styles: Vec::new(),
            height: 0,
            rasters: Vec::new(),
        };
        let start = self
            .blocks
            .partition_point(|block| block.start + block.height <= first);
        for block in self.blocks[start..]
            .iter()
            .take_while(|block| block.start < end)
        {
            let from = first.saturating_sub(block.start);
            let to = (end - block.start).min(block.height);
            for row in from..to {
                let y = u16::try_from(block.start + row - first).expect("visible document row");
                let prefix = if row == 0 || block.format.is_code() {
                    block.prefix.clone()
                } else {
                    block
                        .prefix
                        .chars()
                        .map(|c| if c == '>' { '>' } else { ' ' })
                        .collect()
                };
                let prefix_role = if block.format.is_code() {
                    Role::CodeLabel
                } else {
                    Role::Quote
                };
                let mut glyphs: Vec<_> = flow_text(&prefix, self.width)
                    .expect("validated prefix")
                    .glyphs
                    .into_iter()
                    .map(|glyph| MarkdownGlyph {
                        point: Point::new(glyph.point.x, y),
                        grapheme: glyph.grapheme,
                        decoration: Decoration::role(prefix_role),
                        hyperlink: None,
                    })
                    .collect();
                let shown = block.pages.window(row, NonZeroU16::MIN);
                let base = block.pages.window_offset(row);
                let flow =
                    flow_text(shown, block.body_width).expect("validated single display row");
                let mut row_role = if block.format == BlockFormat::CodeEdge {
                    Role::CodeLabel
                } else {
                    Role::Code
                };
                for glyph in flow.glyphs {
                    let original = block.pages.source_byte_at(base + glyph.byte_index);
                    let next = block.spans.partition_point(|(start, _)| *start <= original);
                    let decoration = next
                        .checked_sub(1)
                        .and_then(|index| block.spans.get(index))
                        .map_or(Decoration::default(), |(_, d)| *d);
                    row_role = if matches!(decoration.role, Role::Syntax(_)) {
                        Role::Code
                    } else {
                        decoration.role
                    };
                    glyphs.push(MarkdownGlyph {
                        point: Point::new(block.indent + glyph.point.x, y),
                        grapheme: glyph.grapheme,
                        hyperlink: decoration
                            .hyperlink
                            .and_then(|index| self.hyperlinks.get(index))
                            .cloned(),
                        decoration,
                    });
                }
                if block.format.is_code() || block.format == BlockFormat::CodeEdge {
                    result.row_styles.push((y, Decoration::role(row_role)));
                }
                glyphs.sort_by_key(|glyph| (glyph.point.y, glyph.point.x));
                result.glyphs.extend(glyphs);
            }
        }
        result.height = u16::try_from(
            self.height
                .saturating_sub(first)
                .min(usize::from(height.get())),
        )
        .expect("bounded window height");
        result
    }
}
