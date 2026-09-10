//! Markdown-to-cell projection, isolated from transcript storage and terminal output.
//!
//! pulldown-cmark supplies balanced CommonMark events, including incomplete fences.
//! Its default CLI/HTML features are disabled; no terminal or OS integration is used.
//! A hand-written parser would duplicate delimiter/list rules; an AST parser would
//! retain an extra tree. The parser can be replaced behind `prepare` without changing
//! message state, theme resolution, or Surface writes.

mod chart;
mod code;
mod diagram;
mod image;
mod table;

use std::{collections::BTreeMap, num::NonZeroU16};

use chart::{ChartKind, chart_blocks};
use code::highlight_code;
use diagram::diagram_blocks;
use image::{image_blocks, pixel_color};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::{
    surface::{Attributes, Color, Grapheme, Hyperlink, Point, RasterImage, Style},
    text::flow::{TextFlowError, flow_code, flow_prose, flow_text},
    transcript::LinkResolver,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownStyles {
    pub(crate) heading: Style,
    pub(crate) chart: [Style; 4],
    pub(crate) syntax: [Style; 6],
    pub(crate) rich_media: bool,
    // Color-depth evidence must not change when a semantic code color is overridden.
    pub(crate) pixel_color_capability: Color,
    pub(crate) code: Style,
    pub(crate) code_label: Style,
    pub(crate) quote: Style,
    pub(crate) link: Style,
    pub(crate) diff_added: Style,
    pub(crate) diff_removed: Style,
    pub(crate) diff_meta: Style,
}

impl MarkdownStyles {
    pub(super) fn display_glyph(self, decoration: Decoration, original: Grapheme) -> Grapheme {
        let replacement = match decoration.role {
            Role::Pixel(top, bottom)
                if !self.rich_media || self.pixel_color_capability == Color::Default =>
            {
                let luminance = (u32::from(top[0]) + u32::from(bottom[0])) * 299
                    + (u32::from(top[1]) + u32::from(bottom[1])) * 587
                    + (u32::from(top[2]) + u32::from(bottom[2])) * 114;
                Some(
                    [" ", ".", ":", "-", "=", "+", "*", "#", "%", "@"]
                        [((luminance * 9) / 510_000) as usize],
                )
            },
            Role::Bar if !self.rich_media => match original.as_str() {
                "█" => Some("#"),
                "│" => Some("|"),
                _ => None,
            },
            Role::Chart | Role::ChartSeries(_) | Role::ChartOverlap if !self.rich_media => {
                match original.as_str() {
                    "━" | "─" => Some("="),
                    "└" => Some("+"),
                    "│" => Some("|"),
                    "·" => Some("."),
                    "×" => Some("x"),
                    "▁" => Some("1"),
                    "▂" => Some("2"),
                    "▃" => Some("3"),
                    "▄" => Some("4"),
                    "▅" => Some("5"),
                    "▆" => Some("6"),
                    "▇" => Some("7"),
                    "█" => Some("8"),
                    text if text.chars().all(|c| ('⠀'..='⣿').contains(&c)) => {
                        Some(if text == "⠀" { " " } else { "*" })
                    },
                    _ => None,
                }
            },
            _ => None,
        };
        replacement.map_or(original, |text| {
            Grapheme::try_from(text).expect("single ASCII preview cell")
        })
    }

    #[cfg(test)]
    pub(crate) const fn plain(style: Style) -> Self {
        Self {
            heading: style,
            chart: [style; 4],
            syntax: [style; 6],
            rich_media: true,
            pixel_color_capability: style.foreground,
            code: style,
            code_label: style,
            quote: style,
            link: style,
            diff_added: style,
            diff_removed: style,
            diff_meta: style,
        }
    }

    pub(crate) fn diff_line_style(self, line: &str) -> Style {
        self.resolve(Decoration::role(diff_role(line)), self.code)
    }

    pub(super) fn resolve(self, decoration: Decoration, body: Style) -> Style {
        let mut style = match decoration.role {
            Role::Body => body,
            Role::Heading => self.heading,
            Role::Chart | Role::Bar => self.chart[0],
            Role::ChartSeries(index) => self.chart[usize::from(index)],
            Role::Code => self.code,
            Role::Syntax(index) => self.syntax[usize::from(index)],
            Role::Pixel(top, bottom) => Style::new(
                pixel_color(top, self.pixel_color_capability),
                pixel_color(bottom, self.pixel_color_capability),
                Attributes::empty(),
            ),
            Role::CodeLabel => self.code_label,
            Role::Quote | Role::ChartOverlap => self.quote,
            Role::Link => self.link,
            Role::DiffAdded => self.diff_added,
            Role::DiffRemoved => self.diff_removed,
            Role::DiffMeta => self.diff_meta,
        };
        style.attributes = style.attributes.union(decoration.attributes);
        style
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Role {
    #[default]
    Body,
    Heading,
    Chart,
    ChartSeries(u8),
    ChartOverlap,
    Bar,
    Code,
    Syntax(u8),
    Pixel([u8; 3], [u8; 3]),
    CodeLabel,
    Quote,
    Link,
    DiffAdded,
    DiffRemoved,
    DiffMeta,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Decoration {
    role: Role,
    attributes: Attributes,
    hyperlink: Option<usize>,
}

impl Decoration {
    const fn role(role: Role) -> Self {
        Self {
            role,
            attributes: Attributes::empty(),
            hyperlink: None,
        }
    }
}

pub(super) struct MarkdownGlyph {
    pub(super) point: Point,
    pub(super) grapheme: Grapheme,
    pub(super) decoration: Decoration,
    pub(super) hyperlink: Option<Hyperlink>,
}

pub(super) struct PreparedMarkdown {
    pub(super) glyphs: Vec<MarkdownGlyph>,
    pub(super) row_styles: Vec<(u16, Decoration)>,
    pub(super) height: u16,
    pub(super) rasters: Vec<RasterImage>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum BlockFormat {
    #[default]
    Prose,
    Code,
    CodeEdge,
    Diff,
    TableRow,
}

impl BlockFormat {
    const fn is_code(self) -> bool {
        matches!(self, Self::Code | Self::Diff)
    }
}

#[derive(Clone, Default)]
struct Block {
    text: String,
    spans: Vec<(usize, Decoration)>,
    prefix: String,
    format: BlockFormat,
    gap: bool,
    raster: Option<RasterImage>,
}

enum DocumentBlock {
    Text(Block),
    Table(table::Table),
    Chart(Block, ChartKind),
    Diagram(Block),
    Image {
        source: String,
        alt: String,
        context: Block,
    },
}

#[derive(Default)]
struct Document {
    code_padding: u16,
    link_resolver: Option<LinkResolver>,
    blocks: Vec<DocumentBlock>,
    table: Option<table::Table>,
    current: Block,
    decoration: Decoration,
    stack: Vec<Decoration>,
    lists: Vec<Option<u64>>,
    items: Vec<(String, bool)>,
    quotes: usize,
    format: BlockFormat,
    gap: bool,
    links: Vec<(String, usize)>,
    hyperlink_targets: Vec<Hyperlink>,
    language: String,
    image: Option<(String, String)>,
}

#[cfg(test)]
pub(super) fn prepare(source: &str, width: NonZeroU16) -> Result<PreparedMarkdown, TextFlowError> {
    prepare_with_images(source, width, true, NonZeroU16::new(64).unwrap())
}

#[cfg(test)]
pub(super) fn prepare_with_images(
    source: &str,
    width: NonZeroU16,
    show_images: bool,
    image_max_width: NonZeroU16,
) -> Result<PreparedMarkdown, TextFlowError> {
    prepare_with_media(source, width, show_images, image_max_width, true)
}

#[cfg(test)]
pub(super) fn prepare_with_media(
    source: &str,
    width: NonZeroU16,
    show_images: bool,
    image_max_width: NonZeroU16,
    show_diagrams: bool,
) -> Result<PreparedMarkdown, TextFlowError> {
    prepare_with_links(
        source,
        width,
        show_images,
        image_max_width,
        show_diagrams,
        None,
        1,
    )
}

pub(super) fn prepare_with_links(
    source: &str,
    width: NonZeroU16,
    show_images: bool,
    image_max_width: NonZeroU16,
    show_diagrams: bool,
    resolver: Option<&LinkResolver>,
    code_padding: u16,
) -> Result<PreparedMarkdown, TextFlowError> {
    let mut document = Document {
        link_resolver: resolver.cloned(),
        code_padding: code_padding.min(width.get().saturating_sub(2)),
        ..Document::default()
    };
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES;
    for event in Parser::new_ext(source, options) {
        document.event(event);
    }
    document.flush();
    prepare_document(document, width, show_images, image_max_width, show_diagrams)
}

fn diff_role(line: &str) -> Role {
    if line.starts_with("+++ ")
        || line.starts_with("+++\t")
        || line.starts_with("--- ")
        || line.starts_with("---\t")
        || line.starts_with("@@")
        || line.starts_with("diff ")
        || line.starts_with("index ")
    {
        Role::DiffMeta
    } else if line.starts_with('+') {
        Role::DiffAdded
    } else if line.starts_with('-') {
        Role::DiffRemoved
    } else {
        Role::Code
    }
}

// Feed a literal diff directly to the code renderer. Embedded Markdown fences
// and image URLs in source files must never become executable presentation markup.
pub(super) fn prepare_diff(
    source: &str,
    width: NonZeroU16,
    code_padding: u16,
) -> Result<PreparedMarkdown, TextFlowError> {
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
    prepare_document(document, width, false, NonZeroU16::new(64).unwrap(), false)
}

fn prepare_document(
    document: Document,
    width: NonZeroU16,
    show_images: bool,
    image_max_width: NonZeroU16,
    show_diagrams: bool,
) -> Result<PreparedMarkdown, TextFlowError> {
    let mut result = PreparedMarkdown {
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
        rasters: Vec::new(),
    };
    let mut blocks = Vec::new();
    for block in document.blocks {
        match block {
            DocumentBlock::Text(block) => blocks.push(block),
            DocumentBlock::Diagram(block) => {
                blocks.extend(diagram_blocks(block, width, show_diagrams))
            },
            DocumentBlock::Table(table) => blocks.extend(table.into_blocks(width)?),
            DocumentBlock::Chart(block, trend) => blocks.extend(chart_blocks(block, width, trend)),
            DocumentBlock::Image {
                source,
                alt,
                context,
            } => {
                if show_images {
                    blocks.extend(image_blocks(&source, &alt, context, width, image_max_width));
                } else {
                    let title = if alt.is_empty() { "Image" } else { &alt };
                    blocks.push(media_text(
                        format!("Image · {title}\nImage display disabled"),
                        &context,
                        Role::Quote,
                    ));
                }
            },
        }
    }
    for block in blocks {
        if block.gap && result.height > 0 {
            result.height = result
                .height
                .checked_add(1)
                .ok_or(TextFlowError::HeightOverflow)?;
        }
        // Keep at least two cells for wide body graphemes. On a one-cell body,
        // retain the existing explicit GraphemeTooWide failure.
        let prefix_limit = width.get().saturating_sub(2);
        let prefix: String = block
            .prefix
            .chars()
            .take(usize::from(prefix_limit))
            .collect();
        let indent = u16::try_from(prefix.chars().count())
            .expect("Markdown prefixes are single-cell glyphs");
        if let Some(mut raster) = block.raster.clone() {
            raster.area.origin = Point::new(indent, result.height);
            result.rasters.push(raster);
        }
        let right_padding = if block.format.is_code() {
            (width.get() - indent)
                .saturating_sub(4)
                .min(document.code_padding)
        } else {
            0
        };
        let body_width = NonZeroU16::new(width.get() - indent - right_padding)
            .expect("code padding reserves at least two body cells");
        let text = if block.format.is_code() {
            block.text.strip_suffix('\n').unwrap_or(&block.text)
        } else {
            &block.text
        };
        let flow = if block.format.is_code() {
            flow_code(text, body_width)?
        } else if block.format != BlockFormat::Prose {
            flow_text(text, body_width)?
        } else {
            flow_prose(text, body_width)?
        };
        let height = flow.height.max(1);
        let end = result
            .height
            .checked_add(height)
            .ok_or(TextFlowError::HeightOverflow)?;
        let mut glyphs = Vec::new();
        for row in 0..height {
            let prefix = if row == 0 || block.format.is_code() {
                prefix.clone()
            } else {
                prefix
                    .chars()
                    .map(|c| if c == '>' { '>' } else { ' ' })
                    .collect()
            };
            for glyph in flow_text(&prefix, width)?.glyphs {
                glyphs.push(MarkdownGlyph {
                    point: Point::new(glyph.point.x, result.height + row),
                    grapheme: glyph.grapheme,
                    hyperlink: None,
                    decoration: Decoration::role(if block.format.is_code() {
                        Role::CodeLabel
                    } else {
                        Role::Quote
                    }),
                });
            }
        }
        let mut row_roles = vec![
            if block.format == BlockFormat::CodeEdge {
                Role::CodeLabel
            } else {
                Role::Code
            };
            usize::from(height)
        ];
        let mut span = 0;
        for glyph in flow.glyphs {
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
                .map_or(Decoration::default(), |(_, d)| *d);
            row_roles[usize::from(glyph.point.y)] = if matches!(decoration.role, Role::Syntax(_)) {
                Role::Code
            } else {
                decoration.role
            };
            glyphs.push(MarkdownGlyph {
                point: Point::new(indent + glyph.point.x, result.height + glyph.point.y),
                grapheme: glyph.grapheme,
                hyperlink: decoration
                    .hyperlink
                    .and_then(|index| document.hyperlink_targets.get(index))
                    .cloned(),
                decoration,
            });
        }
        if block.format.is_code() || block.format == BlockFormat::CodeEdge {
            result
                .row_styles
                .extend(row_roles.into_iter().enumerate().map(|(row, role)| {
                    (
                        result.height + u16::try_from(row).expect("bounded code row"),
                        Decoration::role(role),
                    )
                }));
        }
        glyphs.sort_by_key(|glyph| (glyph.point.y, glyph.point.x));
        result.glyphs.extend(glyphs);
        result.height = end;
    }
    Ok(result)
}

// Remove sentence punctuation and unmatched closing delimiters, retaining URL pairs
// such as Wikipedia's Function_(mathematics) and bracketed IPv6 hosts.
fn trim_web_punctuation(text: &str) -> &str {
    let pairs = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];
    let mut balance = [0_i64; 4];
    for c in text.chars() {
        for (index, (open, close)) in pairs.iter().enumerate() {
            if c == *open {
                balance[index] += 1;
            }
            if c == *close {
                balance[index] -= 1;
            }
        }
    }
    let mut end = text.len();
    for c in text.chars().rev() {
        let trim = if let Some(index) = pairs.iter().position(|(_, close)| c == *close) {
            if balance[index] >= 0 {
                false
            } else {
                balance[index] += 1;
                true
            }
        } else {
            matches!(c, ',' | '.' | ';' | '!' | '\'' | '"')
        };
        if !trim {
            break;
        }
        end -= c.len_utf8();
    }
    &text[..end]
}

impl Document {
    fn append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.current.text.is_empty() {
            self.begin_block();
        }
        let decoration = if self.format.is_code() {
            Decoration::role(Role::Code)
        } else {
            self.decoration
        };
        if self
            .current
            .spans
            .last()
            .is_none_or(|(_, last)| *last != decoration)
        {
            self.current
                .spans
                .push((self.current.text.len(), decoration));
        }
        self.current.text.push_str(text);
    }

    // Annotate complete decoded blocks, before wrapping or table normalization.
    // Event::Text fragments can split a URL at HTML entities or emphasis boundaries.
    fn annotate_web_urls(&mut self, block: &mut Block) {
        if block.format != BlockFormat::Prose {
            return;
        }
        let mut candidates = Vec::new();
        let mut offset = 0;
        for token in block.text.split_inclusive(char::is_whitespace) {
            let leading = token.trim_start_matches(|c| {
                matches!(
                    c,
                    '(' | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | ','
                        | '.'
                        | ';'
                        | '!'
                        | '\''
                        | '"'
                )
            });
            let start = offset + token.len() - leading.len();
            let candidate = trim_web_punctuation(leading.trim_end_matches(char::is_whitespace));
            if let Some(target) = Hyperlink::new(candidate) {
                candidates.push((start..start + candidate.len(), target));
            }
            offset += token.len();
        }
        if candidates.is_empty() {
            return;
        }
        let mut spans = block.spans.iter().copied().collect::<BTreeMap<_, _>>();
        spans.entry(0).or_default();
        for (range, target) in candidates {
            let start_style = *spans.range(..=range.start).next_back().unwrap().1;
            let protected = |style: Decoration| {
                style.hyperlink.is_some() || matches!(style.role, Role::Link | Role::Code)
            };
            if protected(start_style)
                || spans
                    .range(range.clone())
                    .any(|(_, style)| protected(*style))
            {
                continue;
            }
            let end_style = *spans.range(..=range.end).next_back().unwrap().1;
            spans.entry(range.start).or_insert(start_style);
            spans.entry(range.end).or_insert(end_style);
            let index = self.hyperlink_targets.len();
            self.hyperlink_targets.push(target);
            for (_, style) in spans.range_mut(range) {
                style.hyperlink = Some(index);
                style.role = Role::Link;
                style.attributes = style.attributes.union(Attributes::UNDERLINE);
            }
        }
        block.spans = spans.into_iter().collect();
    }

    fn begin_block(&mut self) {
        if self.table.is_none() {
            self.current.prefix = "> ".repeat(self.quotes);
            for (marker, used) in &mut self.items {
                self.current.prefix.push_str(&if *used {
                    " ".repeat(marker.len())
                } else {
                    marker.clone()
                });
                *used = true;
            }
        }
        if self.format.is_code() {
            self.current
                .prefix
                .push_str(&" ".repeat(usize::from(self.code_padding)));
        }
        self.current.format = self.format;
        self.current.gap = std::mem::take(&mut self.gap);
    }

    fn flush(&mut self) {
        if !self.current.text.is_empty() {
            let mut block = std::mem::take(&mut self.current);
            self.annotate_web_urls(&mut block);
            if block.format == BlockFormat::Diff {
                block.spans.clear();
                let mut offset = 0;
                for line in block.text.split_inclusive('\n') {
                    let role = diff_role(line);
                    block.spans.push((offset, Decoration::role(role)));
                    offset += line.len();
                }
            }
            if block.format == BlockFormat::Code && self.language == "mermaid" {
                self.blocks.push(DocumentBlock::Diagram(block));
            } else if block.format == BlockFormat::Code
                && matches!(
                    self.language.as_str(),
                    "chart"
                        | "sparkline"
                        | "linechart"
                        | "stepchart"
                        | "histogram"
                        | "scatterchart"
                )
            {
                self.blocks.push(DocumentBlock::Chart(
                    block,
                    match self.language.as_str() {
                        "sparkline" => ChartKind::Sparkline,
                        "linechart" => ChartKind::Line,
                        "stepchart" => ChartKind::Step,
                        "histogram" => ChartKind::Histogram,
                        "scatterchart" => ChartKind::Scatter,
                        _ => ChartKind::Bars,
                    },
                ));
            } else {
                if block.format == BlockFormat::Code {
                    highlight_code(&mut block, &self.language);
                }
                self.blocks.push(DocumentBlock::Text(block));
            }
        }
    }

    fn push_style(&mut self, role: Option<Role>, attributes: Attributes) {
        self.stack.push(self.decoration);
        if let Some(role) = role {
            self.decoration.role = role;
        }
        self.decoration.attributes = self.decoration.attributes.union(attributes);
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Table(alignments) => {
                    self.flush();
                    // Resolve the table's surrounding quote/list prefix once. Cells
                    // own only their inline content and styles.
                    self.begin_block();
                    let context = std::mem::take(&mut self.current);
                    self.table = Some(table::Table::new(alignments, context.prefix, context.gap));
                },
                Tag::TableHead => self.push_style(Some(Role::Heading), Attributes::BOLD),
                Tag::TableRow | Tag::TableCell => {},
                Tag::FootnoteDefinition(label) => {
                    self.flush();
                    self.gap = !self.blocks.is_empty();
                    self.push_style(Some(Role::Quote), Attributes::BOLD);
                    self.append(&format!("[{label}]"));
                    self.flush();
                    self.decoration = self.stack.pop().unwrap_or_default();
                    self.items.push(("  ".to_owned(), true));
                },
                Tag::Paragraph => self.flush(),
                Tag::Heading { .. } => {
                    self.flush();
                    self.push_style(Some(Role::Heading), Attributes::BOLD);
                },
                Tag::BlockQuote(_) => {
                    self.flush();
                    self.quotes += 1;
                },
                Tag::List(start) => {
                    self.flush();
                    self.lists.push(start);
                },
                Tag::Item => {
                    self.flush();
                    self.gap = false;
                    let marker = match self.lists.last_mut() {
                        Some(Some(next)) => {
                            let marker = format!("{next}. ");
                            *next = next.saturating_add(1);
                            marker
                        },
                        _ => "- ".to_owned(),
                    };
                    self.items.push((marker, false));
                },
                Tag::CodeBlock(kind) => {
                    self.flush();
                    self.language = match &kind {
                        CodeBlockKind::Fenced(info) => info
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.gap = !self.blocks.is_empty();
                    self.push_style(Some(Role::CodeLabel), Attributes::empty());
                    if !matches!(self.language.as_str(), "text" | "txt" | "plaintext") {
                        self.append(&" ".repeat(usize::from(self.code_padding)));
                        self.append(match &kind {
                            CodeBlockKind::Fenced(info) => {
                                match info.split_whitespace().next().unwrap_or("code") {
                                    "chart" => "bar chart",
                                    "linechart" => "line chart",
                                    "stepchart" => "step chart",
                                    "histogram" => "histogram",
                                    "scatterchart" => "scatter plot",
                                    "sparkline" => "sparkline",
                                    other => other,
                                }
                            },
                            CodeBlockKind::Indented => "code",
                        });
                        self.current.format = BlockFormat::CodeEdge;
                        self.flush();
                    }
                    self.format = if matches!(&kind, CodeBlockKind::Fenced(info)
                        if matches!(info.split_whitespace().next(), Some("diff" | "patch")))
                    {
                        BlockFormat::Diff
                    } else {
                        BlockFormat::Code
                    };
                },
                Tag::Emphasis => self.push_style(None, Attributes::ITALIC),
                Tag::Strong => self.push_style(None, Attributes::BOLD),
                Tag::Strikethrough => self.push_style(None, Attributes::STRIKETHROUGH),
                Tag::Image { dest_url, .. } if self.table.is_none() => {
                    self.flush();
                    self.begin_block();
                    self.image = Some((dest_url.into_string(), String::new()));
                },
                link @ (Tag::Link { .. } | Tag::Image { .. }) => {
                    let (dest_url, explicit_link) = match link {
                        Tag::Link { dest_url, .. } => (dest_url, true),
                        Tag::Image { dest_url, .. } => (dest_url, false),
                        _ => unreachable!("link or table image matched above"),
                    };
                    let resolved = explicit_link
                        .then(|| {
                            self.link_resolver
                                .as_ref()
                                .and_then(|resolver| resolver.resolve(&dest_url))
                        })
                        .flatten();
                    let hyperlink = resolved
                        .or_else(|| Hyperlink::new(&dest_url))
                        .map(|target| {
                            let index = self.hyperlink_targets.len();
                            self.hyperlink_targets.push(target);
                            index
                        });
                    self.links
                        .push((dest_url.into_string(), self.current.text.len()));
                    self.push_style(Some(Role::Link), Attributes::UNDERLINE);
                    self.decoration.hyperlink = hyperlink;
                },
                _ => {},
            },
            Event::End(tag) => match tag {
                TagEnd::TableCell => {
                    let mut cell = std::mem::take(&mut self.current);
                    self.annotate_web_urls(&mut cell);
                    self.table
                        .as_mut()
                        .expect("table cells belong to a table")
                        .push_cell(cell);
                },
                TagEnd::TableHead | TagEnd::TableRow => {
                    self.table
                        .as_mut()
                        .expect("table rows belong to a table")
                        .finish_row();
                    if tag == TagEnd::TableHead {
                        self.decoration = self.stack.pop().unwrap_or_default();
                    }
                },
                TagEnd::Table => {
                    self.blocks.push(DocumentBlock::Table(
                        self.table.take().expect("balanced table"),
                    ));
                    self.gap = true;
                },
                TagEnd::Paragraph | TagEnd::HtmlBlock => {
                    self.flush();
                    self.gap = self.items.is_empty();
                },
                TagEnd::Heading(_) => {
                    self.flush();
                    self.gap = true;
                    self.decoration = self.stack.pop().unwrap_or_default();
                },
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.quotes = self.quotes.saturating_sub(1);
                    self.gap = true;
                },
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                    self.gap = self.lists.is_empty();
                },
                TagEnd::FootnoteDefinition => {
                    self.flush();
                    self.items.pop();
                    self.gap = true;
                },
                TagEnd::Item => {
                    self.flush();
                    self.items.pop();
                },
                TagEnd::CodeBlock => {
                    if self.current.text.is_empty()
                        && matches!(
                            self.language.as_str(),
                            "chart"
                                | "sparkline"
                                | "linechart"
                                | "stepchart"
                                | "histogram"
                                | "scatterchart"
                                | "mermaid"
                        )
                    {
                        self.append(" ");
                    }
                    self.flush();
                    self.format = BlockFormat::Prose;
                    self.decoration = Decoration::role(Role::Code);
                    self.append(" ");
                    self.current.format = BlockFormat::CodeEdge;
                    self.flush();
                    self.gap = true;
                    self.decoration = self.stack.pop().unwrap_or_default();
                },
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    self.decoration = self.stack.pop().unwrap_or_default();
                },
                TagEnd::Image if self.image.is_some() => {
                    let (source, alt) = self.image.take().expect("balanced image");
                    self.blocks.push(DocumentBlock::Image {
                        source,
                        alt,
                        context: std::mem::take(&mut self.current),
                    });
                    self.gap = true;
                },
                TagEnd::Link | TagEnd::Image => {
                    if let Some((url, start)) = self.links.pop()
                        && !url.is_empty()
                        && self.current.text.get(start..) != Some(url.as_str())
                    {
                        if url.starts_with("data:image/") {
                            self.append(" (embedded image)");
                        } else {
                            self.append(&format!(" ({url})"));
                        }
                    }
                    self.decoration = self.stack.pop().unwrap_or_default();
                },
                _ => {},
            },
            Event::Text(text) if self.image.is_some() => {
                if let Some((_, alt)) = &mut self.image {
                    alt.push_str(&text);
                }
            },
            Event::Text(text) if !self.format.is_code() && text.contains("data:image/") => {
                // Partial Markdown image syntax must not print a growing base64 payload.
                let start = text.find("data:image/").expect("matched image prefix");
                self.append(&text[..start]);
                self.append("Image data loading…");
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => self.append(&text),
            Event::Code(text) if self.image.is_some() => {
                if let Some((_, alt)) = &mut self.image {
                    alt.push_str(&text);
                }
            },
            Event::Code(text) => {
                self.push_style(Some(Role::Code), Attributes::empty());
                self.append(&text);
                self.decoration = self.stack.pop().unwrap_or_default();
            },
            Event::SoftBreak => self.append(" "),
            Event::HardBreak => self.append("\n"),
            Event::Rule => {
                self.flush();
                self.append("---");
                self.flush();
                self.gap = true;
            },
            Event::TaskListMarker(checked) => self.append(if checked { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(label) => {
                let reference = format!("[{label}]");
                if let Some((_, alt)) = &mut self.image {
                    alt.push_str(&reference);
                } else {
                    self.push_style(Some(Role::Quote), Attributes::BOLD);
                    self.append(&reference);
                    self.decoration = self.stack.pop().unwrap_or_default();
                }
            },
            // Math is not enabled, so the parser retains its source syntax.
            Event::InlineMath(text) | Event::DisplayMath(text) => self.append(&text),
        }
    }
}

#[cfg(test)]
mod tests;

fn media_text(text: String, context: &Block, role: Role) -> Block {
    Block {
        text,
        spans: vec![(0, Decoration::role(role))],
        prefix: context.prefix.clone(),
        format: if matches!(role, Role::Chart | Role::Bar) {
            BlockFormat::TableRow
        } else {
            BlockFormat::Prose
        },
        gap: false,
        raster: None,
    }
}
