//! Prompt-specific framing, padding, and glyph projection.

use std::num::NonZeroU16;

use crate::surface::{Grapheme, Point, Size, Style, SurfaceView, WriteOutcome};

const PREFIX_WIDTH: u16 = 2;
const FRAME_ROWS: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptGlyphs {
    marker: &'static str,
    rule: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptStyles {
    pub(crate) body: Style,
    pub(crate) marker: Style,
    pub(crate) rule: Style,
    pub(crate) glyphs: PromptGlyphs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PromptChrome {
    decorated: bool,
    frame_enabled: bool,
    outer_width: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PromptViewport {
    pub(super) content_origin: Point,
    pub(super) content_size: Size,
    framed: bool,
}

impl PromptGlyphs {
    pub(crate) const fn rich() -> Self {
        Self {
            marker: "›",
            rule: "─",
        }
    }

    pub(crate) const fn ascii() -> Self {
        Self {
            marker: ">",
            rule: "-",
        }
    }
}

impl PromptChrome {
    pub(super) fn new(width: NonZeroU16) -> Self {
        Self {
            decorated: width.get() > PREFIX_WIDTH,
            frame_enabled: true,
            outer_width: width,
        }
    }

    pub(super) const fn with_frame(mut self, enabled: bool) -> Self {
        self.frame_enabled = enabled;
        self
    }

    pub(super) fn content_width(self) -> NonZeroU16 {
        if self.decorated {
            NonZeroU16::new(self.outer_width.get() - PREFIX_WIDTH)
                .expect("decorated prompt width leaves one content cell")
        } else {
            self.outer_width
        }
    }

    pub(super) const fn outer_width(self) -> NonZeroU16 {
        self.outer_width
    }

    pub(super) const fn desired_height(self, content_height: NonZeroU16) -> NonZeroU16 {
        let height = if self.decorated && self.frame_enabled {
            content_height.get().saturating_add(FRAME_ROWS)
        } else {
            content_height.get()
        };
        NonZeroU16::new(height).expect("prompt content height is nonzero")
    }

    pub(super) fn viewport(self, height: NonZeroU16) -> PromptViewport {
        let framed = self.decorated && self.frame_enabled && height.get() > FRAME_ROWS;
        let frame_inset = u16::from(framed);
        PromptViewport {
            content_origin: Point::new(u16::from(self.decorated) * PREFIX_WIDTH, frame_inset),
            content_size: Size::new(
                self.content_width().get(),
                height.get() - frame_inset * FRAME_ROWS,
            ),
            framed,
        }
    }

    pub(super) fn paint(
        self,
        view: &mut SurfaceView<'_>,
        viewport: PromptViewport,
        glyphs: PromptGlyphs,
        styles: PromptStyles,
        first_visible_row: u16,
    ) {
        if viewport.framed {
            let last_row = view.size().height - 1;
            paint_rule(view, 0, glyphs.rule, styles.rule);
            paint_rule(view, last_row, glyphs.rule, styles.rule);
        }
        if self.decorated && first_visible_row == 0 {
            let marker = Grapheme::try_from(glyphs.marker)
                .expect("built-in prompt markers are renderable graphemes");
            if view.write(
                Point::new(0, viewport.content_origin.y),
                marker,
                styles.marker,
            ) == WriteOutcome::Clipped
            {
                unreachable!("prompt marker fits its reserved prefix");
            }
        }
    }
}

fn paint_rule(view: &mut SurfaceView<'_>, row: u16, glyph: &str, style: Style) {
    let glyph = Grapheme::try_from(glyph).expect("built-in prompt rules are renderable graphemes");
    for column in 0..view.size().width {
        if view.write(Point::new(column, row), glyph.clone(), style) == WriteOutcome::Clipped {
            unreachable!("one-cell prompt rule glyph fits the prompt view");
        }
    }
}
